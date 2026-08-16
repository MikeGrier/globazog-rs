// Copyright (c) 2026 Mike Grier

use crate::syntax::anchor::{literal_of, literal_prefix};
use crate::syntax::{Pattern, PatternSegment, Token};

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

fn lit(s: &str) -> Vec<Token> {
    s.chars().map(|c| Token::Literal(c as u32)).collect()
}

#[test]
fn literal_of_pure_literal() {
    assert_eq!(literal_of(&lit("src")), Some(cps("src")));
}

#[test]
fn literal_of_rejects_wildcards() {
    let mut seg = lit("a");
    seg.push(Token::Star);
    assert_eq!(literal_of(&seg), None);
    assert_eq!(literal_of(&[Token::Any]), None);
    assert_eq!(literal_of(&[Token::Alt(vec![lit("a")])]), None);
}

#[test]
fn prefix_stops_at_wildcard_segment() {
    // src/foo/*.rs
    let mut star_seg = vec![Token::Star, Token::Literal('.' as u32)];
    star_seg.extend(lit("rs"));
    let pat = Pattern {
        segments: vec![
            PatternSegment::Match(lit("src")),
            PatternSegment::Match(lit("foo")),
            PatternSegment::Match(star_seg),
        ],
    };
    let (prefix, idx) = literal_prefix(&pat);
    assert_eq!(prefix, vec![cps("src"), cps("foo")]);
    assert_eq!(idx, 2);
}

#[test]
fn prefix_stops_at_double_star() {
    // src/**/*
    let pat = Pattern {
        segments: vec![
            PatternSegment::Match(lit("src")),
            PatternSegment::DoubleStar,
            PatternSegment::Match(vec![Token::Star]),
        ],
    };
    let (prefix, idx) = literal_prefix(&pat);
    assert_eq!(prefix, vec![cps("src")]);
    assert_eq!(idx, 1);
}

#[test]
fn all_literal_prefix() {
    let pat = Pattern {
        segments: vec![
            PatternSegment::Match(lit("a")),
            PatternSegment::Match(lit("b")),
        ],
    };
    let (prefix, idx) = literal_prefix(&pat);
    assert_eq!(prefix, vec![cps("a"), cps("b")]);
    assert_eq!(idx, 2);
}
