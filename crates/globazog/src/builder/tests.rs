// Copyright (c) 2026 Mike Grier

use super::*;
use crate::predicate::{Leaf, MetaMask};
use crate::syntax::CaseSensitivity;
use crate::syntax::dialect::Dialect;
use crate::syntax::parse::Anchor;

fn empty_emit() -> Vec<Leaf> {
    Vec::new()
}

#[test]
fn relative_pattern_binds_to_explicit_root() {
    let q = QueryBuilder::new()
        .root("/home/user")
        .pattern("**/*.rs", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new("/home/user")]);
    assert_eq!(q.patterns.len(), 1);
    assert_eq!(q.patterns[0].glob.anchor, Anchor::Relative);
    assert_eq!(q.patterns[0].glob.case, CaseSensitivity::Sensitive);
}

#[test]
fn relative_pattern_without_root_errors() {
    let err = QueryBuilder::new()
        .pattern("*.rs", Dialect::Posix, empty_emit())
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Pattern(_)));
}

#[test]
fn empty_query_errors() {
    let err = QueryBuilder::new().build().unwrap_err();
    assert!(matches!(err, Error::Pattern(_)));
}

#[cfg(unix)]
#[test]
fn posix_absolute_pattern_peels_literal_prefix_into_root() {
    let q = QueryBuilder::new()
        .pattern("/etc/systemd/*.conf", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    // The leading literal segments `etc` and `systemd` peel into the root.
    assert_eq!(q.roots, vec![Root::new("/etc/systemd")]);
    assert_eq!(q.patterns.len(), 1);
    // Only the wildcard remainder stays in the relative pattern.
    assert_eq!(q.patterns[0].glob.pattern.segments.len(), 1);
    assert_eq!(q.patterns[0].glob.anchor, Anchor::Root);
}

#[cfg(unix)]
#[test]
fn posix_absolute_stops_peeling_at_first_wildcard() {
    let q = QueryBuilder::new()
        .pattern("/var/*/cache/**", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new("/var")]);
    // `*`, `cache`, `**` remain relative.
    assert_eq!(q.patterns[0].glob.pattern.segments.len(), 3);
}

#[cfg(unix)]
#[test]
fn posix_absolute_all_literal_keeps_final_segment() {
    let q = QueryBuilder::new()
        .pattern("/etc/hosts", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    // Root at the parent dir and keep the file name as the relative pattern, so the
    // engine enumerates `/etc` and matches `hosts` instead of rooting at the file.
    assert_eq!(q.roots, vec![Root::new("/etc")]);
    assert_eq!(q.patterns[0].glob.pattern.segments.len(), 1);
}

#[test]
fn case_override_applies() {
    let q = QueryBuilder::new()
        .root("/data")
        .pattern_cased(
            "*.TXT",
            Dialect::Posix,
            CaseSensitivity::Insensitive,
            empty_emit(),
        )
        .build()
        .unwrap();
    assert_eq!(q.patterns[0].glob.case, CaseSensitivity::Insensitive);
}

#[test]
fn query_level_default_case_applies_when_pattern_gives_none() {
    let opts = Options {
        default_case: Some(CaseSensitivity::Insensitive),
        ..Options::default()
    };
    let q = QueryBuilder::new()
        .root("/data")
        .options(opts)
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.patterns[0].glob.case, CaseSensitivity::Insensitive);
}

#[cfg(unix)]
#[test]
fn duplicate_derived_roots_are_deduped() {
    let q = QueryBuilder::new()
        .pattern("/opt/app/*.log", Dialect::Posix, empty_emit())
        .pattern("/opt/app/*.tmp", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new("/opt/app")]);
    assert_eq!(q.patterns.len(), 2);
}

#[cfg(unix)]
#[test]
fn anchored_pattern_scopes_to_its_own_root_only() {
    let q = QueryBuilder::new()
        .root("/tmp")
        .pattern("*.txt", Dialect::Posix, empty_emit()) // relative → explicit root /tmp
        .pattern("/etc/*.conf", Dialect::Posix, empty_emit()) // anchored → /etc
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new("/tmp"), Root::new("/etc")]);
    // The relative pattern applies to the explicit root (index 0).
    assert_eq!(q.patterns[0].roots, vec![0]);
    // The anchored pattern applies only to its derived root (/etc = index 1), D-38.
    assert_eq!(q.patterns[1].roots, vec![1]);
}

#[test]
fn fetch_mask_unions_emit_descend_and_result_shape() {
    let q = QueryBuilder::new()
        .root("/data")
        .pattern(
            "*.log",
            Dialect::Posix,
            vec![Leaf::Size {
                op: crate::predicate::Cmp::Gt,
                value: 0,
            }],
        )
        .descend(vec![Leaf::IsReparse { negate: true }])
        .result_shape(MetaMask::MTIME)
        .build()
        .unwrap();
    let mask = q.fetch_mask();
    assert!(mask.contains(MetaMask::SIZE)); // from emit
    assert!(mask.contains(MetaMask::REPARSE)); // from descend
    assert!(mask.contains(MetaMask::MTIME)); // from result shape
    assert!(!mask.contains(MetaMask::ATTRS));
}

#[test]
fn follow_links_defaults_to_never_and_is_settable() {
    let q = QueryBuilder::new()
        .root("/data")
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.options.follow_links, FollowLinks::Never);

    let q = QueryBuilder::new()
        .root("/data")
        .follow_links(FollowLinks::Always)
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.options.follow_links, FollowLinks::Always);
}

#[cfg(windows)]
#[test]
fn win_drive_absolute_peels_into_drive_root() {
    let q = QueryBuilder::new()
        .pattern(r"C:\Users\*\Documents", Dialect::Win, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new(r"C:\Users")]);
    assert_eq!(q.patterns[0].glob.anchor, Anchor::Drive('C'));
}

#[cfg(windows)]
#[test]
fn win_leading_separator_without_base_errors() {
    // A leading-separator `win` pattern is current-drive-relative; without a base
    // that is process-global state we refuse to read (D-32), so it must error.
    let r = QueryBuilder::new()
        .pattern(r"\foo\*.txt", Dialect::Win, empty_emit())
        .build();
    assert!(r.is_err());
}

#[cfg(windows)]
#[test]
fn win_leading_separator_with_base_roots_at_base_drive() {
    let q = QueryBuilder::new()
        .base(r"D:\work")
        .pattern(r"\foo\*.txt", Dialect::Win, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new(r"D:\foo")]);
}

#[cfg(not(windows))]
#[test]
fn win_dialect_unsupported_off_windows() {
    let err = QueryBuilder::new()
        .pattern("*.txt", Dialect::Win, empty_emit())
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Pattern(_)));
}

#[test]
fn duplicate_supplied_roots_are_deduped() {
    // `.root(p).root(p)` must scan the tree once, not twice (D-37).
    let q = QueryBuilder::new()
        .root("/data")
        .root("/data")
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots, vec![Root::new("/data")]);
    assert_eq!(q.patterns[0].roots, vec![0]);
}

#[test]
fn lexically_equal_roots_are_deduped() {
    // `.`-fold (D-35): `/data` and `/data/.` are the same root.
    let q = QueryBuilder::new()
        .root("/data")
        .root("/data/.")
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots.len(), 1);
}

#[test]
fn nested_supplied_roots_are_rejected() {
    // D-73/M11: one supplied root nested under another is an error (dropping it would
    // change the match set; the enumerate-once merge is deferred).
    let r = QueryBuilder::new()
        .root("/tmp")
        .root("/tmp/sub")
        .pattern("*.c", Dialect::Posix, empty_emit())
        .build();
    assert!(matches!(r, Err(Error::Options(_))));
    // Order-independent: the descendant supplied first is rejected too.
    let r = QueryBuilder::new()
        .root("/tmp/sub")
        .root("/tmp")
        .pattern("*.c", Dialect::Posix, empty_emit())
        .build();
    assert!(matches!(r, Err(Error::Options(_))));
}

#[test]
fn sibling_supplied_roots_are_allowed() {
    let q = QueryBuilder::new()
        .root("/a")
        .root("/b")
        .pattern("*.c", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots.len(), 2);
}

#[test]
fn parent_dir_in_root_is_rejected() {
    // `..` is rejected, not resolved (D-26/D-35), so overlap detection stays sound.
    let r = QueryBuilder::new()
        .root("/a/../b")
        .pattern("*.c", Dialect::Posix, empty_emit())
        .build();
    assert!(matches!(r, Err(Error::Options(_))));
}

#[cfg(windows)]
#[test]
fn windows_roots_dedup_case_insensitively() {
    // D-28: Windows paths are case-insensitive, so `C:/Data` and `C:/data` are one
    // root.
    let q = QueryBuilder::new()
        .root("C:/Data")
        .root("C:/data")
        .pattern("*.c", Dialect::Posix, empty_emit())
        .build()
        .unwrap();
    assert_eq!(q.roots.len(), 1);
}

#[test]
fn zero_ring_capacity_is_rejected() {
    let opts = Options {
        ring_capacity: 0,
        ..Options::default()
    };
    let r = QueryBuilder::new()
        .root("/data")
        .options(opts)
        .pattern("*.txt", Dialect::Posix, empty_emit())
        .build();
    assert!(matches!(r, Err(Error::Options(_))));
}

#[test]
fn relative_base_is_rejected() {
    // A relative base would be resolved via the current directory (a process global);
    // a leading-separator pattern must be given an absolute base (D-32).
    let r = QueryBuilder::new()
        .base("relative/dir")
        .pattern("/foo/*.c", Dialect::Posix, empty_emit())
        .build();
    assert!(r.is_err());
}
