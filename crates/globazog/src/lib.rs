// Copyright (c) 2026 Mike Grier

//! `globazog` — a high-performance, Windows-first globbing and directory-traversal
//! library.
//!
//! A query is a **set of glob patterns over a set of roots** (D-36). You build one
//! with [`QueryBuilder`], call [`submit`](QueryBuilder::submit), and service the
//! resulting **completion ring**: matches, container enter/end markers, per-entry
//! errors, and a final terminal marker all arrive as [`CqItem`]s in delivery order.
//! The walk runs on a permit-bounded pool of background threads (D-3); the client's
//! drain rate is the backpressure throttle (D-11).
//!
//! ```no_run
//! use globazog::{CqItem, Dialect, QueryBuilder};
//!
//! // Every `*.rs` under the current directory, recursively.
//! let handle = QueryBuilder::new()
//!     .root(".")
//!     .pattern("**/*.rs", Dialect::Posix, Vec::new())
//!     .submit()
//!     .expect("valid query");
//!
//! let ring = handle.completions();
//! loop {
//!     match ring.wait_pop() {
//!         CqItem::Match(m) => println!("match: {}", m.name.to_string_lossy()),
//!         CqItem::Terminal(_) => break,
//!         _ => {}
//!     }
//! }
//! ```
//!
//! # Dialects and brace escaping (D-45)
//!
//! Patterns are parsed in a named [`Dialect`]. [`Dialect::Posix`] uses `/`
//! separators and `\` escapes; [`Dialect::Win`] uses both `/` and `\` as separators
//! (so `\` cannot escape) and instead escapes a literal brace by **doubling** it:
//! `{{` means a literal `{` and `}}` means a literal `}`, while a single `{a,b}` is
//! alternation. For example, the `win` pattern `logs\{{2026}}\*.txt` matches files
//! in a directory literally named `{2026}`.
//!
//! # Design
//!
//! The full design is recorded in `DESIGN-NOTES.md` at the repository root; each
//! module's docs cite the relevant decision IDs (`D-n`), and the engine's model is
//! decision `D-70`.

pub mod builder;
pub mod error;
pub mod predicate;
pub mod ring;
pub mod syntax;
pub mod sys;

mod engine;

pub use builder::{Options, PatternEntry, Query, QueryBuilder, QueryHandle, Root};
pub use error::{EntryError, Error};
pub use predicate::{Cmp, EntryType, Leaf, MetaMask, TimeField};
pub use ring::{
    CompletionRing, ContainerEnd, ContainerEnter, ContainerId, ContainerName, CqError, CqItem,
    Decision, DecisionRequest, DecisionToken, EntryMetaOwned, Match, Name, PatternMask,
    SubmissionQueue, Terminal, TerminalReason,
};
pub use syntax::CaseSensitivity;
pub use syntax::dialect::Dialect;
