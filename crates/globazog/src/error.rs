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

    /// An I/O failure while establishing the query (e.g. opening a root).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// A per-entry or per-subtree failure, surfaced as an item in the output stream
/// (D-53). It never aborts the walk — traversal continues past it.
///
// The owning container is attached once the ring item types land (D-64).
#[derive(Debug, Error)]
#[error("entry error: {source}")]
#[non_exhaustive]
pub struct EntryError {
    /// The underlying OS error for the failed operation.
    #[source]
    pub source: std::io::Error,
}
