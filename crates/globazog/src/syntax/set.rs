// Copyright (c) 2026 Mike Grier

//! The pattern-set model (D-36–D-41): many patterns matched over one traversal.
//! Mixed dialects are allowed (D-41). `matches` reports which patterns matched
//! (D-40); `should_descend` is the union descend decision (D-39) driven by a sound
//! mid-pattern viability test (D-34). Anchor/root derivation is applied by the
//! builder (M6); this set matches on the relative segment IR.

use crate::error::Error;
use crate::syntax::dialect::Dialect;
use crate::syntax::matcher::{match_path, match_segment};
use crate::syntax::parse::{Anchor, parse};
use crate::syntax::{CaseSensitivity, CodePoint, Pattern, PatternSegment};

#[cfg(test)]
mod tests;

/// A compiled pattern within a set: its IR, source dialect, anchor, and case rule.
#[derive(Clone, Debug)]
pub struct CompiledPattern {
    /// The compiled segment IR.
    pub pattern: Pattern,
    /// How the pattern text was rooted (D-33).
    pub anchor: Anchor,
    /// The dialect the pattern was parsed with.
    pub dialect: Dialect,
    /// The effective case-sensitivity (dialect default or caller override, D-23).
    pub case: CaseSensitivity,
}

/// A set of patterns answered by a single traversal (D-36).
#[derive(Clone, Debug, Default)]
pub struct PatternSet {
    patterns: Vec<CompiledPattern>,
}

impl PatternSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// A set built from already-compiled patterns (the builder's lowered output),
    /// preserving their order so match indices align with the caller's pattern list.
    pub fn from_compiled(patterns: Vec<CompiledPattern>) -> Self {
        Self { patterns }
    }

    /// Parse and add a pattern; returns its index in the set. `case` overrides the
    /// dialect default (D-23) when `Some`.
    pub fn add(
        &mut self,
        input: &str,
        dialect: Dialect,
        case: Option<CaseSensitivity>,
    ) -> Result<usize, Error> {
        let parsed = parse(input, dialect)?;
        let idx = self.patterns.len();
        self.patterns.push(CompiledPattern {
            pattern: parsed.pattern,
            anchor: parsed.anchor,
            dialect,
            case: case.unwrap_or_else(|| dialect.default_case()),
        });
        Ok(idx)
    }

    /// The number of patterns in the set.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// The compiled pattern at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&CompiledPattern> {
        self.patterns.get(index)
    }

    /// Indices of every pattern that matches `path` (D-40). The ring layer (M6)
    /// materializes these as a bitset.
    pub fn matches(&self, path: &[&[CodePoint]]) -> Vec<usize> {
        self.patterns
            .iter()
            .enumerate()
            .filter(|(_, p)| match_path(&p.pattern.segments, path, p.case))
            .map(|(i, _)| i)
            .collect()
    }

    /// Whether to descend into a directory whose path-from-root is `dir` — the
    /// union across the set (D-39): descend if any pattern could match below.
    pub fn should_descend(&self, dir: &[&[CodePoint]]) -> bool {
        self.patterns
            .iter()
            .any(|p| descend_viable(&p.pattern.segments, dir, p.case))
    }

    /// Like [`should_descend`](Self::should_descend) but only over patterns whose
    /// index satisfies `applies` — used to scope the descend decision to the patterns
    /// that apply to the current root, so an anchored pattern does not force descent
    /// under an unrelated root (D-38).
    pub fn should_descend_where(
        &self,
        dir: &[&[CodePoint]],
        applies: impl Fn(usize) -> bool,
    ) -> bool {
        self.patterns
            .iter()
            .enumerate()
            .filter(|(i, _)| applies(*i))
            .any(|(_, p)| descend_viable(&p.pattern.segments, dir, p.case))
    }
}

/// Whether some non-empty continuation of `dir` could match `pat` — the sound
/// mid-pattern viability test that drives descend pruning (D-34, D-39).
fn descend_viable(pat: &[PatternSegment], dir: &[&[CodePoint]], case: CaseSensitivity) -> bool {
    let mut pat = pat;
    let mut dir = dir;
    loop {
        match pat.split_first() {
            // Pattern exhausted exactly at this depth: nothing deeper can match.
            None => return false,
            // `**` absorbs any number of further segments: descent is always viable.
            Some((PatternSegment::DoubleStar, _)) => return true,
            Some((PatternSegment::Match(seg), rest)) => match dir.split_first() {
                Some((head, tail)) => {
                    if match_segment(seg, head, case) {
                        pat = rest;
                        dir = tail;
                    } else {
                        return false;
                    }
                }
                // Directory consumed with a segment still to match below: descend.
                None => return true,
            },
        }
    }
}
