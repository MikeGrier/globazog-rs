// Copyright (c) 2026 Mike Grier

//! Path structure over code points (D-25, D-26): splitting into segments on a
//! dialect-supplied separator predicate, collapsing consecutive separators, and
//! `.` / `..` classification.

use crate::syntax::CodePoint;

#[cfg(test)]
mod tests;

const DOT: CodePoint = '.' as u32;

/// Split `cps` into non-empty segments on any code point for which `is_sep` returns
/// true. Consecutive separators collapse — empty segments are dropped — implementing
/// D-25. The leading-`\\` UNC exception is handled by the `win` dialect (M3).
pub fn split_segments<F>(cps: &[CodePoint], is_sep: F) -> Vec<&[CodePoint]>
where
    F: Fn(CodePoint) -> bool,
{
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &cp) in cps.iter().enumerate() {
        if is_sep(cp) {
            if let Some(s) = start.take() {
                out.push(&cps[s..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push(&cps[s..]);
    }
    out
}

/// True if a segment is `.` — a no-op component, stripped from patterns (D-26).
pub fn is_dot(seg: &[CodePoint]) -> bool {
    seg == [DOT]
}

/// True if a segment is `..` — rejected in patterns (D-26).
pub fn is_dotdot(seg: &[CodePoint]) -> bool {
    seg == [DOT, DOT]
}
