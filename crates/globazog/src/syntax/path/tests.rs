// Copyright (c) 2026 Mike Grier

use crate::syntax::path::{is_dot, is_dotdot, split_segments};

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

fn slash(cp: u32) -> bool {
    cp == '/' as u32
}

fn owned(v: Vec<&[u32]>) -> Vec<Vec<u32>> {
    v.into_iter().map(<[u32]>::to_vec).collect()
}

#[test]
fn simple_split() {
    let p = cps("a/b/c");
    assert_eq!(
        owned(split_segments(&p, slash)),
        vec![cps("a"), cps("b"), cps("c")]
    );
}

#[test]
fn collapse_consecutive_separators() {
    let p = cps("a//b");
    assert_eq!(owned(split_segments(&p, slash)), vec![cps("a"), cps("b")]);
}

#[test]
fn leading_and_trailing_separators_dropped() {
    let p = cps("/a/b/");
    assert_eq!(owned(split_segments(&p, slash)), vec![cps("a"), cps("b")]);
}

#[test]
fn empty_and_all_separators() {
    assert!(split_segments(&cps(""), slash).is_empty());
    assert!(split_segments(&cps("///"), slash).is_empty());
}

#[test]
fn dot_classification() {
    assert!(is_dot(&cps(".")));
    assert!(!is_dot(&cps("..")));
    assert!(!is_dot(&cps("a")));
}

#[test]
fn dotdot_classification() {
    assert!(is_dotdot(&cps("..")));
    assert!(!is_dotdot(&cps(".")));
    assert!(!is_dotdot(&cps("...")));
}

#[test]
fn multi_separator_predicate() {
    // Both `/` and `\` as separators (win-style).
    let sep = |c: u32| c == '/' as u32 || c == '\\' as u32;
    let p = cps("a\\b/c");
    assert_eq!(
        owned(split_segments(&p, sep)),
        vec![cps("a"), cps("b"), cps("c")]
    );
}
