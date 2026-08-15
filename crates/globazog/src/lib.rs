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
//! # Servicing the completion ring
//!
//! [`QueryHandle::completions`] gives you the [`CompletionRing`]. Pop items with
//! [`wait_pop`](CompletionRing::wait_pop) (blocking), [`pop`](CompletionRing::pop)
//! (non-blocking), or [`drain`](CompletionRing::drain) (all queued), and keep going
//! until a [`CqItem::Terminal`] arrives — it is always the last item. The variants:
//!
//! - [`CqItem::Match`] — an entry matched. [`Match::name`] is the entry name,
//!   [`Match::matched`] is a [`PatternMask`] whose `.iter()` yields the indices of
//!   the patterns that matched (in the order they were added), and [`Match::meta`]
//!   carries the requested [size / timestamps / type](EntryMetaOwned).
//! - [`CqItem::ContainerEnter`] / [`CqItem::ContainerEnd`] — a directory scan
//!   started / its subtree finished. Ends cascade bottom-up and are 1:1 with their
//!   enters; an enter always precedes any `Match` or child that references it.
//! - [`CqItem::Error`] — a per-entry or per-directory failure (e.g. permission
//!   denied, or an entry that vanished mid-scan); the walk continues past it (D-53).
//!   A single unreadable entry never discards its readable siblings.
//! - [`CqItem::Terminal`] — the walk ended:
//!   [`Completed`](TerminalReason::Completed) (ran to the end),
//!   [`Cancelled`](TerminalReason::Cancelled) (a [`cancel`](QueryHandle::cancel) was
//!   honored), or [`Failed`](TerminalReason::Failed) (a fatal error — currently a
//!   root that could not be enumerated — stopped the walk; the causing error is the
//!   [`CqItem::Error`] immediately before this terminal, D-71).
//!
//! Full paths are not shipped per entry; reconstruct them client-side by keeping a
//! `container id → (parent, name)` map from the [`ContainerEnter`] stream and
//! walking to the root (a [`ContainerName::Root`] carries the root's index). For a
//! flat listing you often only need [`Match::name`], as the example above shows.
//!
//! # Filtering, backpressure, and cancellation
//!
//! Each pattern takes a per-pattern **emit** filter (the third argument to
//! [`pattern`](QueryBuilder::pattern)) and the query takes one **descend** filter
//! ([`descend`](QueryBuilder::descend)); both are AND-only conjunctions of [`Leaf`]
//! conditions evaluated inline. The ring is **bounded and never drops** — a slow
//! consumer simply throttles the walk (D-11). [`QueryHandle::cancel`] stops early
//! (you still get a `Cancelled` terminal after queued items), and dropping the
//! handle cancels and joins the engine threads. See [`Options`] to tune the permit
//! count, ring capacity, and cycle detection.
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
