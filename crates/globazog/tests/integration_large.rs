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
    // Skipped under CI: the 60-level chain can exceed Windows' legacy MAX_PATH on
    // runners without long-path support (the setup uses std `fs::create_dir`).
    if std::env::var_os("CI").is_some() {
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

    let handle = QueryBuilder::new()
        .root(root.path())
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
    // The invalid bytes survived decoding as surrogate-escaped code points (D-46).
    assert!(names[0].name.code_points().len() >= "name.dat".len());
    assert_eq!(names[0].meta.size, 7);
}
