// Copyright (c) 2026 Mike Grier

//! Glob syntax: the closed set of named dialects (D-15–D-20), each a front-end that
//! lowers a UTF-8 pattern into the shared segment-structured IR (D-18); the matcher
//! over 32-bit code points (D-46) with `*` / `**` / `?` / n-ary brace alternation
//! (D-24, D-44, D-67); anchor extraction and the pattern-set model (D-36–D-39).
//!
//! This module owns the dialect-independent core (IR, matcher, path splitting,
//! anchor extraction) plus the dialect front-ends (`dialect`, `parse`) that lower
//! each named dialect's pattern text into that IR; `upcase` backs case-folding.

pub mod anchor;
pub mod decode;
pub mod dialect;
pub mod matcher;
pub mod parse;
pub mod path;
pub mod set;

mod upcase;

/// A Unicode code point in the matcher's 32-bit space (D-46). Unlike [`char`] this
/// may hold an unpaired surrogate (Windows) or a surrogate-escaped byte (Linux), so
/// it is a raw `u32`, not a `char`.
pub type CodePoint = u32;

/// Whether matching folds case (D-23, D-28). The default is dialect-supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseSensitivity {
    /// Exact code-point comparison.
    Sensitive,
    /// Case-folded comparison via the Windows ordinal uppercase table (D-28).
    Insensitive,
}

/// One token within a single path segment (D-44). `*` and `?` never cross a
/// separator — crossing is [`PatternSegment::DoubleStar`]'s job (D-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A literal code point.
    Literal(CodePoint),
    /// `?` — exactly one code point.
    Any,
    /// `*` — zero or more code points within the segment.
    Star,
    /// `{a,b,…}` — n-ary alternation (D-67); each arm is a token sequence.
    Alt(Vec<Segment>),
}

/// A single path segment's matcher: a sequence of [`Token`]s.
pub type Segment = Vec<Token>;

/// One element of a compiled pattern's segment sequence (D-18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternSegment {
    /// A normal segment matched against exactly one path segment.
    Match(Segment),
    /// `**` — matches zero or more whole path segments (D-24).
    DoubleStar,
}

/// A compiled, dialect-independent glob pattern (D-18): a sequence of segments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The segment sequence, in order from the root.
    pub segments: Vec<PatternSegment>,
}
