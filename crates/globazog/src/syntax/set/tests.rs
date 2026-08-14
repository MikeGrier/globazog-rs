// Copyright (c) 2026 Mike Grier

use crate::syntax::dialect::Dialect;
use crate::syntax::set::PatternSet;

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

#[test]
fn multi_pattern_matches_report_indices() {
    let mut set = PatternSet::new();
    let c = set.add("*.c*", Dialect::Posix, None).unwrap();
    let h = set.add("*.h*", Dialect::Posix, None).unwrap();
    assert_eq!(c, 0);
    assert_eq!(h, 1);

    let main_c = cps("main.c");
    let util_h = cps("util.h");
    let readme = cps("readme.md");
    assert_eq!(set.matches(&[&main_c]), vec![0]);
    assert_eq!(set.matches(&[&util_h]), vec![1]);
    assert!(set.matches(&[&readme]).is_empty());
}

#[test]
fn descend_union_prunes_unrelated_dirs() {
    let mut set = PatternSet::new();
    set.add("src/**/*.rs", Dialect::Posix, None).unwrap();

    let src = cps("src");
    let other = cps("other");
    // At the root, we must descend to look for `src`.
    assert!(set.should_descend(&[]));
    // `src` is viable; `other` is not.
    assert!(set.should_descend(&[&src]));
    assert!(!set.should_descend(&[&other]));
}

#[test]
fn descend_stops_when_pattern_exhausted() {
    let mut set = PatternSet::new();
    set.add("a/b", Dialect::Posix, None).unwrap();

    let a = cps("a");
    let b = cps("b");
    assert!(set.should_descend(&[&a])); // need to reach a/b
    assert!(!set.should_descend(&[&a, &b])); // a/b is a leaf; nothing below
}

#[test]
fn mixed_dialects_in_one_set() {
    let mut set = PatternSet::new();
    set.add("*.rs", Dialect::Posix, None).unwrap();
    set.add("*.RS", Dialect::Win, None).unwrap(); // win is case-insensitive
    let lib = cps("lib.rs");
    // posix (case-sensitive) matches lib.rs; win (case-insensitive) matches too.
    assert_eq!(set.matches(&[&lib]), vec![0, 1]);
}
