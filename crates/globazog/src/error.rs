// Copyright (c) 2026 Mike Grier

//! Error taxonomy: the fatal crate error, and the per-entry error item surfaced in
//! the output stream rather than aborting the walk (D-53).

use thiserror::Error;

/// A fatal error from query construction or submission.
///
/// Distinct from [`EntryError`], which reports a *per-entry* failure in the output
/// stream without aborting the walk (D-53).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A pattern could not be compiled by its dialect front-end.
    #[error("invalid pattern: {0}")]
    Pattern(String),

    /// An execution option was invalid (e.g. a zero `ring_capacity`).
    #[error("invalid option: {0}")]
    Options(String),

    /// An I/O failure while establishing the query. An unopenable *root* is **not**
    /// this: `submit` returns before enumeration begins, so a root that cannot be
    /// enumerated surfaces at runtime as a [`EntryError`] `CqItem::Error` followed by
    /// `TerminalReason::Failed` (D-71).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// A per-entry or per-subtree failure, surfaced as an item in the output stream
/// (D-53). It normally does not abort the walk — traversal continues past it. The one
/// exception is a *fatal* error (a root that cannot be enumerated): the engine wraps
/// it in this type, emits it, and then ends the stream with `TerminalReason::Failed`
/// (D-71), so a consumer should treat a following `Failed` terminal as "stop".
#[derive(Debug, Error)]
#[error("entry error: {source}")]
#[non_exhaustive]
pub struct EntryError {
    /// The failing entry's name, when a specific entry can be attributed. `None` for
    /// a directory-level failure — an unopenable root, or a late listing read that
    /// truncates the directory — where no single entry is at fault.
    pub name: Option<crate::ring::Name>,
    /// The underlying OS error for the failed operation.
    #[source]
    pub source: std::io::Error,
}
