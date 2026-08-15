// Copyright (c) 2026 Mike Grier

use crate::syntax::dialect::Dialect;
use crate::syntax::parse::{Anchor, parse};
use crate::syntax::{PatternSegment, Token};

fn l(c: char) -> Token {
    Token::Literal(c as u32)
}

fn litseg(s: &str) -> PatternSegment {
    PatternSegment::Match(s.chars().map(l).collect())
}

fn seg(ts: Vec<Token>) -> PatternSegment {
    PatternSegment::Match(ts)
}

fn segments(input: &str, d: Dialect) -> Vec<PatternSegment> {
    parse(input, d).unwrap().pattern.segments
}

#[test]
fn posix_simple_relative() {
    let p = parse("a/b/c", Dialect::Posix).unwrap();
    assert_eq!(p.anchor, Anchor::Relative);
    assert_eq!(
        p.pattern.segments,
        vec![litseg("a"), litseg("b"), litseg("c")]
    );
}

#[test]
fn posix_absolute_root() {
    let p = parse("/usr/foo", Dialect::Posix).unwrap();
    assert_eq!(p.anchor, Anchor::Root);
    assert_eq!(p.pattern.segments, vec![litseg("usr"), litseg("foo")]);
}

#[test]
fn posix_collapses_and_trims_separators() {
    assert_eq!(
        segments("a//b/", Dialect::Posix),
        vec![litseg("a"), litseg("b")]
    );
}

#[test]
fn posix_star_extension_segment() {
    assert_eq!(
        segments("*.rs", Dialect::Posix),
        vec![seg(vec![Token::Star, l('.'), l('r'), l('s')])]
    );
}

#[test]
fn posix_double_star_segment() {
    assert_eq!(
        segments("a/**/b", Dialect::Posix),
        vec![litseg("a"), PatternSegment::DoubleStar, litseg("b")]
    );
}

#[test]
fn posix_backslash_escapes_metachar() {
    // "\*" -> a literal star, not a wildcard.
    assert_eq!(segments("\\*", Dialect::Posix), vec![seg(vec![l('*')])]);
}

#[test]
fn posix_dot_segment_stripped() {
    assert_eq!(
        segments("a/./b", Dialect::Posix),
        vec![litseg("a"), litseg("b")]
    );
}

#[test]
fn posix_dotdot_rejected() {
    assert!(parse("a/../b", Dialect::Posix).is_err());
}

#[test]
fn embedded_double_star_rejected() {
    assert!(parse("a**b", Dialect::Posix).is_err());
    assert!(parse("**b", Dialect::Posix).is_err());
}

#[test]
fn brace_alternation() {
    assert_eq!(
        segments("*.{c,h}", Dialect::Posix),
        vec![seg(vec![
            Token::Star,
            l('.'),
            Token::Alt(vec![vec![l('c')], vec![l('h')]]),
        ])]
    );
}

#[test]
fn unterminated_brace_rejected() {
    assert!(parse("{a,b", Dialect::Posix).is_err());
}

#[test]
fn nested_brace_rejected() {
    assert!(parse("{a,{b}}", Dialect::Posix).is_err());
}

#[test]
fn posix_dangling_escape_rejected() {
    assert!(parse("abc\\", Dialect::Posix).is_err());
}

#[test]
fn win_both_separators() {
    let p = parse("a\\b/c", Dialect::Win).unwrap();
    assert_eq!(p.anchor, Anchor::Relative);
    assert_eq!(
        p.pattern.segments,
        vec![litseg("a"), litseg("b"), litseg("c")]
    );
}

#[test]
fn win_unc_anchor() {
    let p = parse("\\\\srv\\share\\x", Dialect::Win).unwrap();
    assert_eq!(p.anchor, Anchor::Unc);
    assert_eq!(
        p.pattern.segments,
        vec![litseg("srv"), litseg("share"), litseg("x")]
    );
}

#[test]
fn win_drive_anchor_uppercased() {
    let p = parse("d:\\foo", Dialect::Win).unwrap();
    assert_eq!(p.anchor, Anchor::Drive('D'));
    assert_eq!(p.pattern.segments, vec![litseg("foo")]);
}

#[test]
fn win_root_anchor() {
    let p = parse("\\foo", Dialect::Win).unwrap();
    assert_eq!(p.anchor, Anchor::Root);
    assert_eq!(p.pattern.segments, vec![litseg("foo")]);
}

#[test]
fn win_brace_doubling_is_literal() {
    // "a{{b}}" -> literal "a{b}".
    assert_eq!(
        segments("a{{b}}", Dialect::Win),
        vec![seg(vec![l('a'), l('{'), l('b'), l('}')])]
    );
}

#[test]
fn win_brace_doubling_not_honored_inside_alternation_arm() {
    // Limitation (D-45): inside an alternation arm, `{{`/`}}` doubling is not
    // recognized. `{` is the (rejected) start of a nested group and the first `}`
    // closes the arm, so a literal brace cannot appear inside `{a,b}`.
    assert!(parse("x{a,b{{c}", Dialect::Win).is_err());
}

#[test]
fn win_brace_alternation_still_works() {
    assert_eq!(
        segments("*.{cpp,h}", Dialect::Win),
        vec![seg(vec![
            Token::Star,
            l('.'),
            Token::Alt(vec![vec![l('c'), l('p'), l('p')], vec![l('h')]]),
        ])]
    );
}
