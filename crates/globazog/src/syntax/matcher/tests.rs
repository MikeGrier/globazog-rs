// Copyright (c) 2026 Mike Grier

use crate::syntax::matcher::{match_path, match_segment};
use crate::syntax::{CaseSensitivity, PatternSegment, Token};

const CS: CaseSensitivity = CaseSensitivity::Sensitive;
const CI: CaseSensitivity = CaseSensitivity::Insensitive;

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

fn lit(s: &str) -> Vec<Token> {
    s.chars().map(|c| Token::Literal(c as u32)).collect()
}

fn m(s: &str) -> PatternSegment {
    PatternSegment::Match(lit(s))
}

#[test]
fn literal_exact() {
    assert!(match_segment(&lit("foo"), &cps("foo"), CS));
    assert!(!match_segment(&lit("foo"), &cps("bar"), CS));
    assert!(!match_segment(&lit("foo"), &cps("foo2"), CS));
}

#[test]
fn empty_pattern_matches_empty_only() {
    assert!(match_segment(&[], &cps(""), CS));
    assert!(!match_segment(&[], &cps("x"), CS));
}

#[test]
fn question_matches_exactly_one() {
    let p = vec![
        Token::Literal('a' as u32),
        Token::Any,
        Token::Literal('c' as u32),
    ];
    assert!(match_segment(&p, &cps("abc"), CS));
    assert!(!match_segment(&p, &cps("ac"), CS));
    assert!(!match_segment(&p, &cps("abbc"), CS));
}

#[test]
fn star_matches_zero_or_more() {
    let p = vec![Token::Star];
    assert!(match_segment(&p, &cps(""), CS));
    assert!(match_segment(&p, &cps("anything"), CS));
}

#[test]
fn star_prefix_suffix() {
    let mut p = vec![Token::Star];
    p.extend(lit(".rs"));
    assert!(match_segment(&p, &cps("main.rs"), CS));
    assert!(match_segment(&p, &cps(".rs"), CS));
    assert!(!match_segment(&p, &cps("main.rss"), CS));
}

#[test]
fn star_in_the_middle() {
    let mut p = lit("a");
    p.push(Token::Star);
    p.extend(lit("c"));
    assert!(match_segment(&p, &cps("ac"), CS));
    assert!(match_segment(&p, &cps("abbbc"), CS));
    assert!(!match_segment(&p, &cps("ab"), CS));
}

#[test]
fn alternation_with_extension() {
    // *.{c,h}
    let mut p = vec![Token::Star, Token::Literal('.' as u32)];
    p.push(Token::Alt(vec![lit("c"), lit("h")]));
    assert!(match_segment(&p, &cps("main.c"), CS));
    assert!(match_segment(&p, &cps("util.h"), CS));
    assert!(!match_segment(&p, &cps("readme.md"), CS));
}

#[test]
fn alternation_multi_arm() {
    let p = vec![Token::Alt(vec![lit("cpp"), lit("cc"), lit("cxx")])];
    assert!(match_segment(&p, &cps("cpp"), CS));
    assert!(match_segment(&p, &cps("cxx"), CS));
    assert!(!match_segment(&p, &cps("c"), CS));
}

#[test]
fn alternation_arm_with_wildcard() {
    // {a*,b}
    let p = vec![Token::Alt(vec![
        vec![Token::Literal('a' as u32), Token::Star],
        lit("b"),
    ])];
    assert!(match_segment(&p, &cps("axyz"), CS));
    assert!(match_segment(&p, &cps("a"), CS));
    assert!(match_segment(&p, &cps("b"), CS));
    assert!(!match_segment(&p, &cps("c"), CS));
}

#[test]
fn case_insensitive_ascii() {
    assert!(match_segment(&lit("README"), &cps("readme"), CI));
    assert!(!match_segment(&lit("README"), &cps("readme"), CS));
}

#[test]
fn case_insensitive_latin1_folds() {
    // 'É' (U+00C9) matches 'é' (U+00E9) via the Windows uppercase table (D-28).
    assert!(match_segment(&[Token::Literal(0x00C9)], &[0x00E9], CI));
    assert!(!match_segment(&[Token::Literal(0x00C9)], &[0x00E9], CS));
}

#[test]
fn case_insensitive_greek_and_cyrillic() {
    // Greek α/Α (U+03B1 / U+0391) and Cyrillic а/А (U+0430 / U+0410).
    assert!(match_segment(&[Token::Literal(0x0391)], &[0x03B1], CI));
    assert!(match_segment(&[Token::Literal(0x0410)], &[0x0430], CI));
}

#[test]
fn case_insensitive_does_not_conflate_base_letters() {
    // No normalization: 'a' (U+0061) != 'á' (U+00E1) even case-insensitively.
    assert!(!match_segment(&[Token::Literal(0x0061)], &[0x00E1], CI));
}

#[test]
fn case_insensitive_identity_above_bmp() {
    // Supplementary code points fold to themselves (identity).
    assert!(match_segment(&[Token::Literal(0x1F600)], &[0x1F600], CI));
    assert!(!match_segment(&[Token::Literal(0x1F600)], &[0x1F601], CI));
}

#[test]
fn path_exact_segments() {
    let pat = vec![m("src"), m("lib")];
    let (src, lib) = (cps("src"), cps("lib"));
    assert!(match_path(&pat, &[&src, &lib], CS));
    assert!(!match_path(&pat, &[&src], CS));
}

#[test]
fn double_star_zero_or_more_segments() {
    // a/**/b
    let pat = vec![m("a"), PatternSegment::DoubleStar, m("b")];
    let (a, b, x) = (cps("a"), cps("b"), cps("x"));
    assert!(match_path(&pat, &[&a, &b], CS));
    assert!(match_path(&pat, &[&a, &x, &b], CS));
    assert!(match_path(&pat, &[&a, &x, &x, &b], CS));
    assert!(!match_path(&pat, &[&a, &x], CS));
}

#[test]
fn leading_double_star_any_depth_including_root() {
    // **/x
    let pat = vec![PatternSegment::DoubleStar, m("x")];
    let (x, a) = (cps("x"), cps("a"));
    assert!(match_path(&pat, &[&x], CS));
    assert!(match_path(&pat, &[&a, &x], CS));
    assert!(!match_path(&pat, &[&x, &a], CS));
}

#[test]
fn trailing_double_star_matches_self_and_below() {
    // a/**
    let pat = vec![m("a"), PatternSegment::DoubleStar];
    let (a, b) = (cps("a"), cps("b"));
    assert!(match_path(&pat, &[&a], CS));
    assert!(match_path(&pat, &[&a, &b], CS));
    assert!(!match_path(&pat, &[&b], CS));
}
