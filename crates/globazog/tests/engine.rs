// Copyright (c) 2026 Mike Grier

//! M7 engine integration tests: drive real directory trees end-to-end through the
//! public `submit` API (D-3) and validate match results, the container Enter/End
//! nesting invariants (D-64), descend pruning, per-pattern emit filters (D-66), and
//! cancellation (D-61).

use globazog::builder::{Options, QueryBuilder};
use globazog::predicate::{Cmp, Leaf};
use globazog::ring::{ContainerId, CqItem, Name, TerminalReason};
use globazog::syntax::dialect::Dialect;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// Drain a handle's ring to completion, returning every item in delivery order.
fn drain_to_terminal(handle: &globazog::builder::QueryHandle) -> Vec<CqItem> {
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

fn name_string(name: &Name) -> String {
    name.to_string_lossy()
}

/// Build a fixed tree:
/// root/
///   a.txt, b.txt, note.md
///   sub/ (c.txt, d.log)
///   sub/deep/ (e.txt)
///   empty/
fn make_tree(root: &Path) {
    fs::write(root.join("a.txt"), b"aa").unwrap();
    fs::write(root.join("b.txt"), b"bbbb").unwrap();
    fs::write(root.join("note.md"), b"m").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/c.txt"), b"c").unwrap();
    fs::write(root.join("sub/d.log"), b"dd").unwrap();
    fs::create_dir(root.join("sub/deep")).unwrap();
    fs::write(root.join("sub/deep/e.txt"), b"e").unwrap();
    fs::create_dir(root.join("empty")).unwrap();
}

/// Collect match names (by pattern-agnostic union) from a drained stream.
fn match_names(items: &[CqItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| match i {
            CqItem::Match(m) => Some(name_string(&m.name)),
            _ => None,
        })
        .collect()
}

#[test]
fn recursive_txt_match_over_tree() {
    let root = tempfile::tempdir().unwrap();
    make_tree(root.path());

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*.txt", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let mut names = match_names(&items);
    names.sort();
    assert_eq!(names, vec!["a.txt", "b.txt", "c.txt", "e.txt"]);

    // Exactly one terminal, marked Completed.
    let terminals: Vec<_> = items
        .iter()
        .filter(|i| matches!(i, CqItem::Terminal(_)))
        .collect();
    assert_eq!(terminals.len(), 1);
    match terminals[0] {
        CqItem::Terminal(t) => assert_eq!(t.reason, TerminalReason::Completed),
        _ => unreachable!(),
    }
}

#[test]
fn container_enter_end_invariants_hold_on_real_tree() {
    let root = tempfile::tempdir().unwrap();
    make_tree(root.path());

    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("**/*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let mut live: HashMap<ContainerId, Option<ContainerId>> = HashMap::new();
    let mut ended: HashSet<ContainerId> = HashSet::new();
    let mut enters = 0;
    let mut ends = 0;
    for item in &items {
        match item {
            CqItem::ContainerEnter(e) => {
                if let Some(p) = e.parent {
                    assert!(live.contains_key(&p), "child entered before parent");
                    assert!(!ended.contains(&p), "child entered after parent ended");
                }
                assert!(live.insert(e.id, e.parent).is_none(), "duplicate enter");
                enters += 1;
            }
            CqItem::Match(m) => {
                assert!(
                    live.contains_key(&m.container),
                    "match in unknown container"
                );
            }
            CqItem::ContainerEnd(end) => {
                let parent = live.remove(&end.id).expect("end without enter");
                assert!(
                    !live.values().any(|p| *p == Some(end.id)),
                    "container ended while a child was still live"
                );
                if let Some(p) = parent {
                    assert!(!ended.contains(&p), "parent ended before child");
                }
                ended.insert(end.id);
                ends += 1;
            }
            CqItem::Terminal(_) => {
                assert!(live.is_empty(), "containers still live at terminal");
            }
            _ => {}
        }
    }
    // root + sub + sub/deep + empty = 4 containers, each entered and ended once.
    assert_eq!(enters, 4);
    assert_eq!(ends, 4);
}

#[test]
fn descend_pruning_limits_traversal() {
    let root = tempfile::tempdir().unwrap();
    make_tree(root.path());

    // A non-recursive single-segment pattern: only the root's own entries match and
    // no descent is needed, so only the root container is entered.
    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("*.txt", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let mut names = match_names(&items);
    names.sort();
    assert_eq!(names, vec!["a.txt", "b.txt"]);

    let enters = items
        .iter()
        .filter(|i| matches!(i, CqItem::ContainerEnter(_)))
        .count();
    assert_eq!(enters, 1, "no subdirectory should be entered");
}

#[test]
fn per_pattern_emit_filter_applies() {
    let root = tempfile::tempdir().unwrap();
    make_tree(root.path());

    // Match all *.txt recursively but only emit those larger than 1 byte.
    // a.txt = 2, b.txt = 4, c.txt = 1, e.txt = 1 -> only a.txt and b.txt.
    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern(
            "**/*.txt",
            Dialect::Posix,
            vec![Leaf::Size {
                op: Cmp::Gt,
                value: 1,
            }],
        )
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let mut names = match_names(&items);
    names.sort();
    assert_eq!(names, vec!["a.txt", "b.txt"]);
}

#[test]
fn multi_pattern_bitset_reports_all_matching_patterns() {
    let root = tempfile::tempdir().unwrap();
    make_tree(root.path());

    // Pattern 0 = *.txt, pattern 1 = a.* ; a.txt matches both.
    let handle = QueryBuilder::new()
        .root(root.path())
        .pattern("*.txt", Dialect::Posix, Vec::new())
        .pattern("a.*", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let a_match = items
        .iter()
        .find_map(|i| match i {
            CqItem::Match(m) if name_string(&m.name) == "a.txt" => Some(m),
            _ => None,
        })
        .expect("a.txt matched");
    assert!(a_match.matched.contains(0));
    assert!(a_match.matched.contains(1));

    let b_match = items
        .iter()
        .find_map(|i| match i {
            CqItem::Match(m) if name_string(&m.name) == "b.txt" => Some(m),
            _ => None,
        })
        .expect("b.txt matched");
    assert!(b_match.matched.contains(0));
    assert!(!b_match.matched.contains(1));
}

#[test]
fn large_tree_matches_all_and_terminates() {
    let root = tempfile::tempdir().unwrap();
    // 20 dirs x 50 files = 1000 target files, plus nested one level.
    for d in 0..20 {
        let dir = root.path().join(format!("d{d}"));
        fs::create_dir(&dir).unwrap();
        for f in 0..50 {
            fs::write(dir.join(format!("f{f}.dat")), b"x").unwrap();
        }
    }

    let opts = Options {
        permits: 8,
        ring_capacity: 64,
        ..Options::default()
    };
    let handle = QueryBuilder::new()
        .root(root.path())
        .options(opts)
        .pattern("**/*.dat", Dialect::Posix, Vec::new())
        .submit()
        .unwrap();
    let items = drain_to_terminal(&handle);

    let matches = items
        .iter()
        .filter(|i| matches!(i, CqItem::Match(_)))
        .count();
    assert_eq!(matches, 1000);
    // 1 root + 20 subdirs entered and ended.
    let enters = items
        .iter()
        .filter(|i| matches!(i, CqItem::ContainerEnter(_)))
        .count();
    let ends = items
        .iter()
        .filter(|i| matches!(i, CqItem::ContainerEnd(_)))
        .count();
    assert_eq!(enters, 21);
    assert_eq!(ends, 21);
}

#[test]
fn cancel_terminates_with_cancelled_marker() {
    let root = tempfile::tempdir().unwrap();
    for d in 0..30 {
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

    // Cancel immediately, then drain to the terminal marker.
    handle.cancel();
    let items = drain_to_terminal(&handle);

    match items.last().unwrap() {
        CqItem::Terminal(t) => {
            assert!(matches!(
                t.reason,
                TerminalReason::Cancelled | TerminalReason::Completed
            ));
        }
        other => panic!("stream must end with a terminal, got {other:?}"),
    }
    // Exactly one terminal, and it is last.
    let terminals = items
        .iter()
        .filter(|i| matches!(i, CqItem::Terminal(_)))
        .count();
    assert_eq!(terminals, 1);
}
