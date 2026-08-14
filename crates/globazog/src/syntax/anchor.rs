// Copyright (c) 2026 Mike Grier

//! Anchor extraction (D-37): peel the leading run of pure-literal segments from a
//! compiled pattern for use as a traversal seek target, and classify a segment as a
//! plain literal for the engine's mid-pattern viability pruning (D-34).

use crate::syntax::{CodePoint, Pattern, PatternSegment, Token};

#[cfg(test)]
mod tests;

/// If `seg` is composed solely of literal tokens (no `*` / `?` / alternation),
/// return its code points; otherwise `None`.
pub fn literal_of(seg: &[Token]) -> Option<Vec<CodePoint>> {
    let mut out = Vec::with_capacity(seg.len());
    for tok in seg {
        match tok {
            Token::Literal(cp) => out.push(*cp),
            _ => return None,
        }
    }
    Some(out)
}

/// The leading literal prefix of a pattern (D-37): the maximal run of pure-literal
/// `Match` segments from the start, as code-point segments, plus the index of the
/// first segment not included (where wildcard matching resumes).
pub fn literal_prefix(pattern: &Pattern) -> (Vec<Vec<CodePoint>>, usize) {
    let mut prefix = Vec::new();
    for (i, seg) in pattern.segments.iter().enumerate() {
        match seg {
            PatternSegment::Match(tokens) => match literal_of(tokens) {
                Some(lit) => prefix.push(lit),
                None => return (prefix, i),
            },
            PatternSegment::DoubleStar => return (prefix, i),
        }
    }
    let end = pattern.segments.len();
    (prefix, end)
}
