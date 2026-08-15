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

#[cfg(not(windows))]
#[test]
fn win_dialect_unsupported_off_windows() {
    let err = QueryBuilder::new()
        .pattern("*.txt", Dialect::Win, empty_emit())
        .build()
        .unwrap_err();
    assert!(matches!(err, Error::Pattern(_)));
}
