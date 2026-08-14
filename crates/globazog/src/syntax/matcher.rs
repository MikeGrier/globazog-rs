// Copyright (c) 2026 Mike Grier

//! Matching in 32-bit code-point space (D-46): single-segment token matching with
//! `*` / `?` / n-ary alternation (D-44), cross-segment `**` (D-24), and ASCII-level
//! case folding (D-28; full Unicode simple folding is tracked in CHECKLIST.md M2-6).

use crate::syntax::{CaseSensitivity, CodePoint, PatternSegment, Segment, Token};

#[cfg(test)]
mod tests;

/// Fold a code point for case-insensitive comparison. ASCII `A`–`Z` fold to
/// lowercase; all other code points (including surrogate-escaped values) are
/// returned unchanged (D-28, interim ASCII fold — see M2-6).
fn fold(cp: CodePoint) -> CodePoint {
    if (0x41..=0x5A).contains(&cp) {
        cp + 0x20
    } else {
        cp
    }
}

fn cp_eq(a: CodePoint, b: CodePoint, cs: CaseSensitivity) -> bool {
    match cs {
        CaseSensitivity::Sensitive => a == b,
        CaseSensitivity::Insensitive => fold(a) == fold(b),
    }
}

/// Match a single segment's tokens against one path segment's code points.
///
/// Uses backtracking; segments are bounded (`NAME_MAX`) and complexity DoS is a
/// non-goal (D-43), so exponential worst cases on pathological alternations are
/// accepted.
pub fn match_segment(tokens: &[Token], input: &[CodePoint], cs: CaseSensitivity) -> bool {
    match tokens.split_first() {
        None => input.is_empty(),
        Some((tok, rest)) => match tok {
            Token::Literal(cp) => {
                !input.is_empty()
                    && cp_eq(*cp, input[0], cs)
                    && match_segment(rest, &input[1..], cs)
            }
            Token::Any => !input.is_empty() && match_segment(rest, &input[1..], cs),
            Token::Star => (0..=input.len()).any(|k| match_segment(rest, &input[k..], cs)),
            Token::Alt(arms) => arms.iter().any(|arm| match_arm_then(arm, rest, input, cs)),
        },
    }
}

/// Match `arm` against a prefix of `input`, then `rest` against the remainder.
fn match_arm_then(arm: &Segment, rest: &[Token], input: &[CodePoint], cs: CaseSensitivity) -> bool {
    (0..=input.len())
        .any(|k| match_segment(arm, &input[..k], cs) && match_segment(rest, &input[k..], cs))
}

/// Match a compiled pattern's segments against a path (a sequence of segments).
/// `**` matches zero or more whole segments (D-24).
pub fn match_path(pattern: &[PatternSegment], path: &[&[CodePoint]], cs: CaseSensitivity) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((PatternSegment::DoubleStar, rest)) => {
            (0..=path.len()).any(|k| match_path(rest, &path[k..], cs))
        }
        Some((PatternSegment::Match(seg), rest)) => {
            !path.is_empty() && match_segment(seg, path[0], cs) && match_path(rest, &path[1..], cs)
        }
    }
}
