// Copyright (c) 2026 Mike Grier

//! M8-2 large-scale integration tests: thousands of files, deep trees, bounded
//! no-drop backpressure, cancellation, symlink loop-safety, and non-UTF-8 names
//! (D-30, D-46). These drive the engine end-to-end through the public API over the
//! native enumeration backend.

use globazog::{CqItem, Dialect, Options, QueryBuilder, QueryHandle, TerminalReason};
use std::fs;

fn drain(handle: &QueryHandle) -> Vec<CqItem> {
    let ring = handle.completions();
    let mut items = Vec::new();
    loop {
        let item = ring.wait_pop();
        let terminal = matches!(item, CqItem::Terminal(_));
        items.push(item);
        if terminal {
            return items;
        }
    }
}

fn count<F: Fn(&CqItem) -> bool>(items: &[CqItem], pred: F) -> usize {
    items.iter().filter(|i| pred(i)).count()
}

fn matches(items: &[CqItem]) -> usize {
    count(items, |i| matches!(i, CqItem::Match(_)))
}

fn enters(items: &[CqItem]) -> usize {
    count(items, |i| matches!(i, CqItem::ContainerEnter(_)))
}

fn ends(items: &[CqItem]) -> usize {
    count(items, |i| matches!(i, CqItem::ContainerEnd(_)))
}

fn terminal(items: &[CqItem]) -> TerminalReason {
    match items.last().unwrap() {
        CqItem::Terminal(t) => t.reason,
        other => panic!("stream must end with a terminal, got {other:?}"),
    }
}

#[test]
fn thousands_of_files_all_match() {
    let root = tempfile::tempdir().unwrap();
    // 50 dirs x 100 files = 5000 target files.
    for d in 0..50 {
        let dir = root.path().join(format!("d{d:02}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..100 {
            fs::write(dir.join(format!("f{f:03}.dat")), b"x").unwrap();
        }
    }

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    assert_eq!(matches(&items), 5000);
    assert_eq!(enters(&items), 51); // root + 50 subdirs
    assert_eq!(ends(&items), 51);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[test]
fn deep_tree_terminates_with_balanced_containers() {
    // Skipped under CI on Windows only: the 60-level chain can exceed Windows' legacy
    // MAX_PATH on runners without long-path support (setup uses std `fs::create_dir`).
    // MAX_PATH is irrelevant on Linux, so that CI still exercises the deep traversal.
    if cfg!(windows) && std::env::var_os("CI").is_some() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    // A single chain 60 levels deep, a file at the bottom.
    let mut p = root.path().to_path_buf();
    for level in 0..60 {
        p = p.join(format!("lvl{level}"));
        fs::create_dir(&p).unwrap();
    }
    fs::write(p.join("bottom.txt"), b"deep").unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/bottom.txt", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    assert_eq!(matches(&items), 1);
    // root + 60 nested dirs = 61 containers, each entered and ended exactly once.
    assert_eq!(enters(&items), 61);
    assert_eq!(ends(&items), 61);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[test]
fn bounded_ring_delivers_everything_under_backpressure() {
    let root = tempfile::tempdir().unwrap();
    for d in 0..20 {
        let dir = root.path().join(format!("d{d}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..50 {
            fs::write(dir.join(format!("f{f}.dat")), b"x").unwrap();
        }
    }

    // A tiny ring forces the workers to park on backpressure while the single
    // consumer drains one item at a time; nothing may be dropped (D-11, D-68).
    let opts = Options {
        permits: 8,
        ring_capacity: 2,
        ..Options::default()
    };
    let handle = QueryBuilder::new()
        .root(root.path())
        .options(opts)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    assert_eq!(matches(&items), 1000);
    assert_eq!(enters(&items), 21);
    assert_eq!(ends(&items), 21);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[test]
fn concurrent_container_ends_do_not_deadlock_on_a_full_ring() {
    // Many containers + a capacity-1 ring + many workers: `ContainerEnd` is a
    // mandatory (cancel-immune) push, so several workers routinely block on a full
    // ring at once. The push must survive a coalesced space wake (the periodic
    // recheck recovers a lost one) — otherwise a worker sleeps forever, joins hang,
    // and the terminal never arrives.
    let root = tempfile::tempdir().unwrap();
    for d in 0..120 {
        let dir = root.path().join(format!("d{d:03}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..3 {
            fs::write(dir.join(format!("f{f}.dat")), b"x").unwrap();
        }
    }

    let opts = Options {
        permits: 16,
        ring_capacity: 1,
        ..Options::default()
    };
    let handle = QueryBuilder::new()
        .root(root.path())
        .options(opts)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    // Every container closed exactly once (120 dirs + root) and the walk completed.
    assert_eq!(enters(&items), 121);
    assert_eq!(ends(&items), 121);
    assert_eq!(matches(&items), 360);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[test]
fn cancel_large_walk_terminates_once() {
    let root = tempfile::tempdir().unwrap();
    for d in 0..40 {
        let dir = root.path().join(format!("d{d}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..50 {
            fs::write(dir.join(format!("f{f}.dat")), b"x").unwrap();
        }
    }

    let opts = Options {
        permits: 4,
        ring_capacity: 8,
        ..Options::default()
    };
    let handle = QueryBuilder::new()
        .root(root.path())
        .options(opts)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();

    handle.cancel();
    let items = drain(&handle);

    // Exactly one terminal, and it is the last item.
    assert_eq!(count(&items, |i| matches!(i, CqItem::Terminal(_))), 1);
    assert!(matches!(
        terminal(&items),
        TerminalReason::Cancelled | TerminalReason::Completed
    ));
}

#[test]
fn mixed_roots_and_patterns() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(a.path().join("one.rs"), b"r").unwrap();
    fs::write(a.path().join("keep.md"), b"m").unwrap();
    fs::write(b.path().join("two.rs"), b"r").unwrap();

    let handle = QueryBuilder::new()
        .root(a.path())
        .root(b.path())
        .pattern("*.rs", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    // Two roots, so two containers; two `.rs` matches across them.
    assert_eq!(matches(&items), 2);
    assert_eq!(enters(&items), 2);
    assert_eq!(ends(&items), 2);
}

#[cfg(unix)]
#[test]
fn symlink_loop_is_not_followed() {
    // A symlink pointing back at the root is a cycle; because symlinks are reported
    // as non-directory reparse entries and never followed, traversal terminates and
    // the symlink target is never entered as a container.
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::write(root.path().join("sub/x.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    // Only root + sub are entered; `loop` is matched but not descended.
    assert_eq!(enters(&items), 2);
    assert_eq!(ends(&items), 2);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    let names: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(m.name.to_string_lossy()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"loop".to_string()));
}

#[cfg(unix)]
#[test]
fn non_utf8_filename_is_enumerated_and_matched() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let name = OsStr::from_bytes(b"bad\xff\xfename.dat");
    fs::write(root.path().join(name), b"payload").unwrap();

    // Request SIZE explicitly so the byte-size assertion below is populated under the
    // D-62 lazy-fetch contract (stat-tier fields are only fetched when asked for).
    let handle = QueryBuilder::new()
        .root(root.path())
        .result_shape(globazog::MetaMask::SIZE)
        .pattern("*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    let names: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 1);
    // The invalid bytes survived decoding as the exact PEP-383 surrogate-escaped code
    // points (D-46), reversibly and in place: 0xff -> U+DCFF, 0xfe -> U+DCFE.
    let expected: Vec<u32> = vec![
        b'b' as u32,
        b'a' as u32,
        b'd' as u32,
        0xDCFF,
        0xDCFE,
        b'n' as u32,
        b'a' as u32,
        b'm' as u32,
        b'e' as u32,
        b'.' as u32,
        b'd' as u32,
        b'a' as u32,
        b't' as u32,
    ];
    assert_eq!(names[0].name.code_points(), expected.as_slice());
    assert_eq!(names[0].meta.size, 7);
}

#[test]
fn unopenable_root_terminates_with_failed() {
    // A root that cannot be enumerated at all is fatal (D-71): the stream ends with
    // Terminal::Failed, preceded by exactly one error item, and the root container
    // is still balanced (one enter, one end).
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("does-not-exist");

    let handle = QueryBuilder::new()
        .root(&missing)
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    assert_eq!(terminal(&items), TerminalReason::Failed);
    assert_eq!(count(&items, |i| matches!(i, CqItem::Error(_))), 1);
    // The root could not be opened, so no container is announced at all (D-64): a
    // handle-open failure yields only an error + fatal terminal, never a phantom
    // enter/end pair.
    assert_eq!(enters(&items), 0);
    assert_eq!(ends(&items), 0);
    // The causing error lands immediately before the terminal (D-71).
    let n = items.len();
    assert!(matches!(items[n - 2], CqItem::Error(_)));
    assert!(matches!(&items[n - 1], CqItem::Terminal(t) if t.reason == TerminalReason::Failed));
    // A root open failure is scoped to no container and names no single entry (D-53),
    // but still reports which root frame failed (D-38).
    let CqItem::Error(e) = &items[n - 2] else {
        unreachable!()
    };
    assert!(e.container.is_none());
    assert!(e.error.name.is_none());
    assert_eq!(e.root, Some(0));
}

#[cfg(unix)]
#[test]
fn symlinked_dir_followed_only_with_follow_always() {
    use globazog::FollowLinks;

    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("f.dat"), b"x").unwrap();
    std::os::unix::fs::symlink("real", root.path().join("link")).unwrap();

    // Default (Never, D-72/D-13): the file is found once — via `real/`, never through
    // the symlink.
    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(matches(&items), 1);
    assert_eq!(terminal(&items), TerminalReason::Completed);

    // Always: found twice — via `real/` and via `link/` — and loop-safe (the walk
    // terminates; the reparse cycle guard bounds it).
    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(matches(&items), 2);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[cfg(unix)]
#[test]
fn follow_always_with_cycle_back_to_root_is_bounded_and_completes() {
    use globazog::FollowLinks;

    // A symlink pointing back at its own root forms a traversal cycle. With
    // FollowLinks::Always the walk enters it, so the D-51 cycle guard (default
    // cycle_detection) must cut the loop — the walk stays bounded and completes
    // rather than exploding or hanging. This is the loop test the `Always` policy
    // was missing (the acyclic case is covered above).
    let root = tempfile::tempdir().unwrap();
    let sub = root.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("f.dat"), b"x").unwrap();
    // `sub/loop` -> the root itself: descending it re-enters the whole tree.
    std::os::unix::fs::symlink(root.path(), sub.join("loop")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);

    // The guard cut the cycle: the walk terminated normally instead of diverging.
    assert_eq!(terminal(&items), TerminalReason::Completed);
    // Bounded container enters — a small finite count, never unbounded growth.
    let enters = items
        .iter()
        .filter(|i| matches!(i, CqItem::ContainerEnter(_)))
        .count();
    assert!(
        enters < 20,
        "container enters should be bounded, got {enters}"
    );
    // Every enter is still matched by exactly one end (D-64), even across the follow.
    let ends = items
        .iter()
        .filter(|i| matches!(i, CqItem::ContainerEnd(_)))
        .count();
    assert_eq!(enters, ends);
    // The real file is still found at least once.
    assert!(matches(&items) >= 1);
}

#[cfg(unix)]
#[test]
fn confine_to_roots_blocks_symlink_escape() {
    use globazog::{BlockReason, FollowLinks};

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.dat"), b"x").unwrap();

    // A real subdir inside the root with its own file.
    let inside = root.path().join("inside");
    fs::create_dir(&inside).unwrap();
    fs::write(inside.join("in.dat"), b"x").unwrap();

    // `escape` leaves the root; `stay` stays within it.
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    std::os::unix::fs::symlink(&inside, root.path().join("stay")).unwrap();

    let dat_names = |items: &[CqItem]| -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                CqItem::Match(m) => Some(m.name.to_string_lossy().to_string()),
                _ => None,
            })
            .collect()
    };

    // Confined + follow: the escaping link is declined (its target's `secret.dat` is
    // never reported) and one `Blocked{RootEscape}` names it; the in-root link is
    // still followed (D-75).
    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    let names = dat_names(&items);
    assert!(
        !names.iter().any(|n| n == "secret.dat"),
        "escaping link was followed under confinement: {names:?}"
    );
    // `in.dat` must appear twice: once via the real `inside` dir and once via the
    // in-root `stay` symlink — proving the confined in-root descent actually happened
    // rather than being skipped.
    assert_eq!(
        names.iter().filter(|n| *n == "in.dat").count(),
        2,
        "in-root symlink was not followed under confinement: {names:?}"
    );
    let blocks: Vec<&CqItem> = items
        .iter()
        .filter(|i| matches!(i, CqItem::Blocked(_)))
        .collect();
    assert_eq!(blocks.len(), 1);
    let CqItem::Blocked(b) = blocks[0] else {
        unreachable!()
    };
    assert_eq!(b.reason, BlockReason::RootEscape);
    assert_eq!(b.name.to_string_lossy(), "escape");

    // Not confined: the escaping link IS followed, `secret.dat` appears, no `Blocked`.
    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    assert!(dat_names(&items).iter().any(|n| n == "secret.dat"));
    assert!(!items.iter().any(|i| matches!(i, CqItem::Blocked(_))));
}

#[cfg(unix)]
#[test]
fn confine_fail_closed_on_broken_symlink() {
    use globazog::{BlockReason, FollowLinks};

    // A dangling symlink: its target does not exist, so `canonicalize` fails and the
    // fail-closed policy declines it as RootEscape rather than following it (D-75).
    let root = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/no/such/globazog/target", root.path().join("broken")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    let blocks: Vec<&CqItem> = items
        .iter()
        .filter(|i| matches!(i, CqItem::Blocked(_)))
        .collect();
    assert_eq!(blocks.len(), 1);
    let CqItem::Blocked(b) = blocks[0] else {
        unreachable!()
    };
    assert_eq!(b.reason, BlockReason::RootEscape);
    assert_eq!(b.name.to_string_lossy(), "broken");
}

#[cfg(unix)]
#[test]
fn confine_is_inert_under_follow_never() {
    // `FollowLinks::Never` (default): reparse points are never followed, so the
    // confinement check never runs — no `Blocked`, and the escaping target is unseen.
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.dat"), b"x").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .confine_to_roots(true)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    assert!(!items.iter().any(|i| matches!(i, CqItem::Blocked(_))));
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, CqItem::Match(m) if m.name.to_string_lossy() == "secret.dat"))
    );
}

#[cfg(unix)]
#[test]
fn confine_boundary_and_multiple_roots() {
    use globazog::{BlockReason, FollowLinks};

    // Two roots. A link into the *second* root is allowed; a link to a sibling whose
    // name string-*prefixes* a root is an escape — containment is component-wise, not a
    // string prefix (D-75).
    let base = tempfile::tempdir().unwrap();
    let root_a = base.path().join("root");
    let root_b = base.path().join("other");
    let sibling = base.path().join("rootsibling"); // string-prefixes "root"
    fs::create_dir(&root_a).unwrap();
    fs::create_dir(&root_b).unwrap();
    fs::create_dir(&sibling).unwrap();
    fs::write(root_b.join("b.dat"), b"x").unwrap();
    fs::write(sibling.join("s.dat"), b"x").unwrap();
    std::os::unix::fs::symlink(&root_b, root_a.join("to_b")).unwrap();
    std::os::unix::fs::symlink(&sibling, root_a.join("to_sib")).unwrap();

    let handle = QueryBuilder::new()
        .root(&root_a)
        .root(&root_b)
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);

    let names: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(m.name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    // `to_b` resolves into a supplied root → allowed; the string-prefix sibling is not.
    // `b.dat` must appear twice: once from scanning `root_b` directly and once through
    // the `to_b` symlink, proving the cross-root descent was actually followed (not just
    // reached via the direct root).
    assert_eq!(
        names.iter().filter(|n| *n == "b.dat").count(),
        2,
        "cross-root symlink was not followed: {names:?}"
    );
    assert!(!names.iter().any(|n| n == "s.dat"));
    let blocked: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Blocked(b) => {
                assert_eq!(b.reason, BlockReason::RootEscape);
                Some(b.name.to_string_lossy().to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(blocked, vec!["to_sib".to_string()]);
}

#[cfg(unix)]
#[test]
fn confine_reports_each_escape_separately() {
    use globazog::FollowLinks;

    // Two distinct escaping links each get their own `Blocked` (no dedup swallowing).
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("esc_a")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("esc_b")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    let mut blocked: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Blocked(b) => Some(b.name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    blocked.sort();
    assert_eq!(blocked, vec!["esc_a".to_string(), "esc_b".to_string()]);
}

#[cfg(unix)]
#[test]
fn confine_reports_hardlinked_symlink_escape_twice() {
    use globazog::FollowLinks;

    // Two hard-linked names for the *same* escaping symlink share one filesystem
    // identity. Confinement is checked before cycle detection consumes that identity,
    // so both names are reported per-entry rather than the second being swallowed as a
    // cycle (D-75).
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link_a = root.path().join("esc_a");
    std::os::unix::fs::symlink(outside.path(), &link_a).unwrap();
    // `link(2)` on Linux does not follow the symlink: this is a second name for the
    // symlink inode itself, giving both names the same file id.
    fs::hard_link(&link_a, root.path().join("esc_b")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);
    let mut blocked: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Blocked(b) => Some(b.name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    blocked.sort();
    assert_eq!(blocked, vec!["esc_a".to_string(), "esc_b".to_string()]);
}

#[cfg(unix)]
#[test]
fn confine_matches_escaping_link_name_but_not_its_target() {
    use globazog::{BlockReason, FollowLinks};

    // The reparse entry itself is unaffected — only the *descent* is declined (D-75) —
    // so an escaping link named `escape` yields both a `Match` for `escape` and a
    // `Blocked{RootEscape}`, while the target's `secret.dat` stays absent.
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.dat"), b"x").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);

    let match_names: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(m.name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    assert!(
        match_names.iter().any(|n| n == "escape"),
        "escaping link name did not match: {match_names:?}"
    );
    assert!(
        !match_names.iter().any(|n| n == "secret.dat"),
        "escaped target contents appeared: {match_names:?}"
    );

    let blocked: Vec<(String, BlockReason)> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Blocked(b) => Some((b.name.to_string_lossy().to_string(), b.reason)),
            _ => None,
        })
        .collect();
    assert_eq!(
        blocked,
        vec![("escape".to_string(), BlockReason::RootEscape)]
    );
}

/// Create a directory junction (a reparse point with the mount-point tag). Unlike a
/// directory symlink, a junction needs no elevated privilege, so it works in a plain
/// CI harness. Returns whether creation succeeded.
#[cfg(windows)]
fn make_junction(link: &std::path::Path, target: &std::path::Path) -> bool {
    std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
#[test]
fn confine_to_roots_blocks_junction_escape() {
    use globazog::{BlockReason, FollowLinks};

    // Windows coverage for the junction / canonical-`\\?\`-prefix + case-fold path
    // (D-75): an escaping junction is declined, an in-root junction is followed.
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.dat"), b"x").unwrap();

    let inside = root.path().join("inside");
    fs::create_dir(&inside).unwrap();
    fs::write(inside.join("in.dat"), b"x").unwrap();

    // Junctions need no elevated privilege, so creation must succeed in the harness;
    // a failure is a real regression, not a reason to skip the coverage.
    assert!(make_junction(&root.path().join("escape"), outside.path()));
    assert!(make_junction(&root.path().join("stay"), &inside));

    let handle = QueryBuilder::new()
        .root(root.path())
        .follow_links(FollowLinks::Always)
        .confine_to_roots(true)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(terminal(&items), TerminalReason::Completed);

    let names: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(m.name.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    assert!(
        !names.iter().any(|n| n == "secret.dat"),
        "escaping junction was followed under confinement: {names:?}"
    );
    // `in.dat` must appear twice: via the real `inside` dir and via the in-root `stay`
    // junction — proving the in-root junction descent actually happened.
    assert_eq!(
        names.iter().filter(|n| *n == "in.dat").count(),
        2,
        "in-root junction was not followed under confinement: {names:?}"
    );

    let blocked: Vec<String> = items
        .iter()
        .filter_map(|i| match i {
            CqItem::Blocked(b) => {
                assert_eq!(b.reason, BlockReason::RootEscape);
                Some(b.name.to_string_lossy().to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(blocked, vec!["escape".to_string()]);
}

#[test]
fn cancellation_still_balances_container_ends() {
    // Even when cancelled mid-flight, every `ContainerEnter` must get a
    // `ContainerEnd` before the terminal (D-64) — a client must never reach the
    // terminal with live (unclosed) containers.
    let root = tempfile::tempdir().unwrap();
    for d in 0..30 {
        let dir = root.path().join(format!("d{d:02}"));
        fs::create_dir(&dir).unwrap();
        for s in 0..5 {
            let sub = dir.join(format!("s{s}"));
            fs::create_dir(&sub).unwrap();
            fs::write(sub.join("f.dat"), b"x").unwrap();
        }
    }

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    handle.cancel();
    let items = drain(&handle);

    let mut enter_ids = Vec::new();
    let mut end_ids = Vec::new();
    for i in &items {
        match i {
            CqItem::ContainerEnter(e) => enter_ids.push(e.id),
            CqItem::ContainerEnd(e) => end_ids.push(e.id),
            _ => {}
        }
    }
    enter_ids.sort();
    end_ids.sort();
    // Same multiset: every enter has exactly one end and vice versa.
    assert_eq!(enter_ids, end_ids);
    assert!(matches!(
        terminal(&items),
        TerminalReason::Cancelled | TerminalReason::Completed
    ));
}

#[cfg(unix)]
#[test]
fn anchored_pattern_does_not_cross_apply_to_other_roots() {
    // D-38: an anchored pattern must match only under its own derived root, never
    // under an unrelated supplied root that happens to contain a matching name.
    let a = tempfile::tempdir().unwrap();
    fs::write(a.path().join("foo.conf"), b"x").unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(b.path().join("bar.conf"), b"x").unwrap();

    // Supplied root `a` (no relative pattern) + an anchored pattern rooted at `b`.
    let bpat = format!("{}/*.conf", b.path().display());
    let handle = QueryBuilder::new()
        .root(a.path())
        .pattern(&bpat, Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    // Only `bar.conf` under `b`; `foo.conf` under `a` is NOT reported.
    assert_eq!(matches(&items), 1);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}

#[cfg(unix)]
#[test]
fn unreferenced_missing_root_is_not_scheduled() {
    // A supplied root that no pattern applies to must not be walked (D-38), so a
    // bogus/unrelated supplied root cannot fail an otherwise-valid anchored traversal
    // with `Failed`.
    let b = tempfile::tempdir().unwrap();
    fs::write(b.path().join("bar.conf"), b"x").unwrap();
    let missing = b.path().join("does-not-exist"); // a bogus explicit root

    let bpat = format!("{}/*.conf", b.path().display());
    let handle = QueryBuilder::new()
        .root(&missing) // unreferenced: only the anchored pattern (rooted at `b`) applies
        .pattern(&bpat, Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain(&handle);
    assert_eq!(matches(&items), 1);
    assert_eq!(terminal(&items), TerminalReason::Completed);
}
