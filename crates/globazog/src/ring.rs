// Copyright (c) 2026 Mike Grier

//! The io_uring-shaped submit/complete API (D-55): the SQ op set (D-66), the CQ
//! item enum (Match / ContainerEnter / ContainerEnd / Error / DecisionRequest /
//! Terminal — D-64) on a bounded MPMC ring (D-68) with park/wake backpressure
//! (D-11) and a waitable-handle + `drain` servicing surface (D-60); cancellation
//! (D-61); Rust-native ABI (D-65).
//!
//! # Representation choices (D-69)
//!
//! - **Name blob:** a CQ item carries the entry name in the crate's reversible
//!   code-point representation ([`crate::syntax::decode`], D-46) rather than raw native code
//!   units. The whole matcher pipeline already operates in code-point space and the
//!   transform is lossless, so this is the "one representation, two readers" of
//!   D-63 realized as the decoded form. A future zero-copy native-blob / inline
//!   fixed-buffer optimization (the true 512-byte descriptor slot, D-68) is
//!   deferred behind profiling.
//! - **Owned items:** [`CqItem`]s are owned and popped by value (D-68). The
//!   borrowed / lending-cursor optimization is deferred.
//! - **Waitable primitive:** ring readiness is signalled through the portable
//!   [`Signal`] (D-60). Raw OS-handle exposure for foreign reactors is a native
//!   follow-up, mirroring the portable-first enumeration backend.

use crate::error::EntryError;
use crate::predicate::{EntryMeta, EntryType, MetaMask};
use crate::syntax::CodePoint;
use crate::sys::signal::Signal;
use crossbeam_queue::ArrayQueue;
use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
mod tests;

/// A container identity within one query's path tree (D-64). Monotonic, non-zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerId(pub NonZeroU64);

/// A `defer-to-client` decision token (D-58, D-64). A separate id space from
/// [`ContainerId`] because a deferred entry may be vetoed and never become a
/// container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecisionToken(pub NonZeroU64);

/// A monotonic non-zero id allocator (one per id space per query, D-64).
#[derive(Debug)]
pub struct IdSpace(AtomicU64);

impl IdSpace {
    /// A fresh id space starting at 1.
    pub fn new() -> Self {
        Self(AtomicU64::new(1))
    }

    /// The next raw id. Panics only on the (practically unreachable) u64 overflow.
    pub fn next_raw(&self) -> NonZeroU64 {
        let v = self.0.fetch_add(1, Ordering::Relaxed);
        NonZeroU64::new(v).expect("id space overflow")
    }

    /// The next container id.
    pub fn next_container(&self) -> ContainerId {
        ContainerId(self.next_raw())
    }

    /// The next decision token.
    pub fn next_decision(&self) -> DecisionToken {
        DecisionToken(self.next_raw())
    }
}

impl Default for IdSpace {
    fn default() -> Self {
        Self::new()
    }
}

/// An owned entry name in the reversible code-point representation (D-46, D-69).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    units: Vec<CodePoint>,
}

impl Name {
    /// Wrap a decoded code-point sequence.
    pub fn from_code_points(units: &[CodePoint]) -> Self {
        Self {
            units: units.to_vec(),
        }
    }

    /// The name's code points (D-46).
    pub fn code_points(&self) -> &[CodePoint] {
        &self.units
    }

    /// A best-effort `String`, mapping any un-mappable code unit to U+FFFD. A
    /// faithful reversible WTF-8 / surrogateescape adapter (D-46) is a later add.
    pub fn to_string_lossy(&self) -> String {
        self.units
            .iter()
            .map(|&c| char::from_u32(c).unwrap_or('\u{FFFD}'))
            .collect()
    }

    /// The number of code units.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether the name is empty.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// A [`ContainerEnter`]'s name (D-64): a named child carries its entry blob, a root
/// carries its index into the submitted roots list (avoids inlining a long root
/// path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerName {
    /// A scanned child directory's own name.
    Entry(Name),
    /// A root: the index into the query's roots list.
    Root(u32),
}

/// A growable set of matched-pattern indices (D-40). Word-backed so a query is not
/// silently capped at 64 patterns; sized to the pattern count at creation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternMask {
    words: Box<[u64]>,
}

impl PatternMask {
    /// An empty mask able to hold `num_patterns` bits.
    pub fn new(num_patterns: usize) -> Self {
        let words = num_patterns.div_ceil(64).max(1);
        Self {
            words: vec![0u64; words].into_boxed_slice(),
        }
    }

    /// A mask over `num_patterns` with the given indices set.
    pub fn from_indices(num_patterns: usize, indices: &[usize]) -> Self {
        let mut m = Self::new(num_patterns);
        for &i in indices {
            m.set(i);
        }
        m
    }

    /// Set bit `index`.
    pub fn set(&mut self, index: usize) {
        self.words[index / 64] |= 1u64 << (index % 64);
    }

    /// Whether bit `index` is set.
    pub fn contains(&self, index: usize) -> bool {
        self.words
            .get(index / 64)
            .is_some_and(|w| w & (1u64 << (index % 64)) != 0)
    }

    /// Whether any bit is set.
    pub fn any(&self) -> bool {
        self.words.iter().any(|&w| w != 0)
    }

    /// The number of set bits.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// The set indices in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            (0..64)
                .filter(move |b| w & (1u64 << b) != 0)
                .map(move |b| wi * 64 + b)
        })
    }
}

/// The owned metadata carried by a [`Match`] / [`DecisionRequest`]. `present`
/// records which stat-tier fields were actually fetched (D-62); name and depth are
/// always valid.
#[derive(Clone, Copy, Debug)]
pub struct EntryMetaOwned {
    /// Which stat-tier fields below are populated (D-62).
    pub present: MetaMask,
    /// Depth from the traversal root (0 = a root's direct child).
    pub depth: u32,
    /// The entry kind.
    pub entry_type: EntryType,
    /// Whether the entry is a reparse point (D-13).
    pub is_reparse: bool,
    /// The reparse tag (0 when not a reparse point).
    pub reparse_tag: u32,
    /// The attribute bitmask.
    pub attributes: u32,
    /// File size in bytes.
    pub size: u64,
    /// Birth / creation time.
    pub btime: i64,
    /// Last-modification time.
    pub mtime: i64,
    /// Last-access time.
    pub atime: i64,
    /// Metadata-change time.
    pub ctime: i64,
}

impl EntryMetaOwned {
    /// Snapshot a borrowed [`EntryMeta`], recording `present` as the fetched fields.
    pub fn from_meta(meta: &EntryMeta, present: MetaMask) -> Self {
        Self {
            present,
            depth: meta.depth,
            entry_type: meta.entry_type,
            is_reparse: meta.is_reparse,
            reparse_tag: meta.reparse_tag,
            attributes: meta.attributes,
            size: meta.size,
            btime: meta.btime,
            mtime: meta.mtime,
            atime: meta.atime,
            ctime: meta.ctime,
        }
    }

    /// Borrow this snapshot back as an [`EntryMeta`] over `name`.
    pub fn borrow<'a>(&self, name: &'a [CodePoint]) -> EntryMeta<'a> {
        EntryMeta {
            name,
            depth: self.depth,
            entry_type: self.entry_type,
            is_reparse: self.is_reparse,
            reparse_tag: self.reparse_tag,
            attributes: self.attributes,
            size: self.size,
            btime: self.btime,
            mtime: self.mtime,
            atime: self.atime,
            ctime: self.ctime,
        }
    }
}

/// A directory scan begins (D-64): permit acquired, handle opened. Emitted before
/// `id` is ever used as a parent, so FIFO delivery gives enter-before-children.
#[derive(Clone, Debug)]
pub struct ContainerEnter {
    /// This container's id.
    pub id: ContainerId,
    /// The parent container, or `None` for a root.
    pub parent: Option<ContainerId>,
    /// This container's name (entry blob, or root index when `parent` is `None`).
    pub name: ContainerName,
}

/// A directory subtree is complete (D-64): the D-9 refcount reached zero. 1:1 with
/// [`ContainerEnter`], emitted as a bottom-up cascade.
#[derive(Clone, Copy, Debug)]
pub struct ContainerEnd {
    /// The container whose subtree is complete.
    pub id: ContainerId,
}

/// A matching entry, emitted at discovery during the parent's enumeration (D-64).
#[derive(Clone, Debug)]
pub struct Match {
    /// The enclosing (parent) container.
    pub container: ContainerId,
    /// The entry's name.
    pub name: Name,
    /// Which patterns matched (D-40).
    pub matched: PatternMask,
    /// The requested inline metadata (D-13, D-62).
    pub meta: EntryMetaOwned,
}

/// A per-entry / per-subtree failure surfaced on the ring rather than aborting the
/// walk (D-53). `container` is the enclosing container when known.
#[derive(Debug)]
pub struct CqError {
    /// The enclosing container, if the failure is attributable to one.
    pub container: Option<ContainerId>,
    /// The underlying failure.
    pub error: EntryError,
}

/// A `defer-to-client` escalation (D-58): the client evaluates the entry on its own
/// thread and answers via [`SqOp::DecisionAnswer`] carrying `token`.
#[derive(Clone, Debug)]
pub struct DecisionRequest {
    /// The correlation token the client echoes in its answer.
    pub token: DecisionToken,
    /// The enclosing container.
    pub container: ContainerId,
    /// The entry's name.
    pub name: Name,
    /// The entry's metadata.
    pub meta: EntryMetaOwned,
}

/// Why the query is terminating (D-61).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalReason {
    /// The traversal ran to completion.
    Completed,
    /// A [`SqOp::Cancel`] was honored.
    Cancelled,
    /// The traversal was stopped by a fatal error (D-71); the causing error is
    /// delivered in a [`CqItem::Error`] that precedes this terminal.
    Failed,
}

/// The final CQ item (D-61): lands after every item already enqueued (FIFO),
/// acknowledging completion or cancellation.
#[derive(Clone, Copy, Debug)]
pub struct Terminal {
    /// Why the query ended.
    pub reason: TerminalReason,
}

/// One completion-queue item (D-64, D-65). Owned; popped by value (D-68).
#[derive(Debug)]
pub enum CqItem {
    /// A directory scan started.
    ContainerEnter(ContainerEnter),
    /// A matching entry was discovered.
    Match(Match),
    /// A directory subtree completed.
    ContainerEnd(ContainerEnd),
    /// A per-entry or per-directory failure; the walk continues past it (D-53). When
    /// it is a fatal error (D-71) it is the last `Error` before a `Terminal::Failed`.
    Error(CqError),
    /// A `defer-to-client` escalation.
    DecisionRequest(DecisionRequest),
    /// The terminal completion/cancellation marker.
    Terminal(Terminal),
}

/// The client's answer to a [`DecisionRequest`] (D-58).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Accept the entry (emit / descend as the query would have).
    Accept,
    /// Veto the entry.
    Reject,
}

/// A submission-queue op that steers a running query (D-66, #4). Booting a query is
/// done directly by [`QueryBuilder::submit`](crate::QueryBuilder::submit) in the
/// synchronous engine; an SQ-driven query-submission (boot) op belongs to the async
/// reactor model and lands with it (M7-6), so it is not represented here yet.
#[derive(Debug)]
pub enum SqOp {
    /// Cancel the running query (D-61).
    Cancel,
    /// Answer a `defer-to-client` request (D-58).
    DecisionAnswer {
        /// The token from the originating [`DecisionRequest`].
        token: DecisionToken,
        /// The client's decision.
        decision: Decision,
    },
}

/// The bounded, no-drop completion ring (D-68): a lock-free MPMC queue of owned
/// [`CqItem`]s with coalesced non-empty signalling for the consumer and a
/// space-available signal for producer backpressure (D-11, D-60). The blocking
/// [`wait_pop`](Self::wait_pop) consumer path stays correct under multiple
/// consumers via wake-propagation in [`pop`](Self::pop) (a departing consumer
/// re-arms the signal while items remain); the blocking producer path
/// ([`push_blocking`](Self::push_blocking)) is single-producer — the engine's
/// multi-producer path uses [`wait_space_timeout`](Self::wait_space_timeout), whose
/// periodic re-check is immune to a collapsed space wake.
#[derive(Debug)]
pub struct CompletionRing {
    queue: ArrayQueue<CqItem>,
    nonempty: Signal,
    space: Signal,
}

impl CompletionRing {
    /// A ring holding up to `capacity` items (must be non-zero).
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0, "ring capacity must be non-zero");
        Self {
            queue: ArrayQueue::new(capacity),
            nonempty: Signal::new(),
            space: Signal::new(),
        }
    }

    /// The ring's fixed capacity.
    pub fn capacity(&self) -> usize {
        self.queue.capacity()
    }

    /// The current number of queued items.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Whether the ring is full.
    pub fn is_full(&self) -> bool {
        self.queue.is_full()
    }

    /// Producer: enqueue an item, signalling the consumer. Returns the item back on
    /// `Err` when the ring is full — the engine then parks its continuation (D-11).
    pub fn try_push(&self, item: CqItem) -> Result<(), CqItem> {
        match self.queue.push(item) {
            Ok(()) => {
                self.nonempty.notify();
                Ok(())
            }
            Err(item) => Err(item),
        }
    }

    /// Producer: enqueue, blocking on the space signal while full (D-11). Suited to
    /// a single producer / tests; the engine uses [`try_push`](Self::try_push) with
    /// its own unified suspension (D-59).
    pub fn push_blocking(&self, item: CqItem) {
        let mut item = item;
        loop {
            match self.try_push(item) {
                Ok(()) => return,
                Err(back) => {
                    item = back;
                    self.space.wait();
                }
            }
        }
    }

    /// Producer: wait up to `timeout` for space to free after a full `try_push`,
    /// returning whether space was signalled. The engine uses this for
    /// cancel-responsive backpressure — it re-checks its cancel flag each time the
    /// wait returns (D-11, D-59).
    pub fn wait_space_timeout(&self, timeout: std::time::Duration) -> bool {
        self.space.wait_timeout(timeout)
    }

    /// Wake any producer parked in [`wait_space_timeout`](Self::wait_space_timeout)
    /// without freeing a slot — used to unblock producers on cancellation (D-61).
    pub fn wake_producers(&self) {
        self.space.notify();
    }

    /// Consumer: pop one item, signalling a producer that space freed (D-11).
    ///
    /// Also re-arms the coalesced non-empty signal when items remain, so the
    /// blocking [`wait_pop`](Self::wait_pop) path is safe with multiple consumers:
    /// several producer notifications can collapse into one bit, but each consumer
    /// that leaves residue wakes the next, so no queued item can strand a waiter.
    pub fn pop(&self) -> Option<CqItem> {
        let item = self.queue.pop();
        if item.is_some() {
            self.space.notify();
            if !self.queue.is_empty() {
                self.nonempty.notify();
            }
        }
        item
    }

    /// Consumer: pop every currently-queued item (the non-blocking `drain`, D-60).
    pub fn drain(&self) -> Vec<CqItem> {
        let mut out = Vec::new();
        while let Some(item) = self.pop() {
            out.push(item);
        }
        out
    }

    /// Consumer: block until at least one item is available, then pop it.
    pub fn wait_pop(&self) -> CqItem {
        loop {
            if let Some(item) = self.pop() {
                return item;
            }
            self.nonempty.wait();
        }
    }

    /// Consumer: block until the ring is non-empty (without popping). A thin
    /// building block for foreign-reactor adapters (D-60).
    pub fn wait_nonempty(&self) {
        while self.queue.is_empty() {
            self.nonempty.wait();
        }
    }
}

/// The submission queue (D-68): low-volume and asymmetric, so a mutex-guarded
/// deque suffices for its three ops (D-66) — no lock-free ring needed.
#[derive(Debug, Default)]
pub struct SubmissionQueue {
    inner: Mutex<VecDeque<SqOp>>,
    signal: Signal,
}

impl SubmissionQueue {
    /// An empty submission queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit an op, waking a servicing engine.
    pub fn submit(&self, op: SqOp) {
        self.inner.lock().unwrap().push_back(op);
        self.signal.notify();
    }

    /// Take the next op without blocking.
    pub fn try_next(&self) -> Option<SqOp> {
        self.inner.lock().unwrap().pop_front()
    }

    /// Take every currently-queued op.
    pub fn drain(&self) -> Vec<SqOp> {
        self.inner.lock().unwrap().drain(..).collect()
    }

    /// Block until an op is available.
    pub fn wait(&self) {
        while self.inner.lock().unwrap().is_empty() {
            self.signal.wait();
        }
    }

    /// Wait up to `timeout` for an op; returns whether one may be available. The
    /// engine's SQ-servicing loop uses this so it can also poll its finished flag.
    pub fn wait_timeout(&self, timeout: std::time::Duration) -> bool {
        if !self.inner.lock().unwrap().is_empty() {
            return true;
        }
        self.signal.wait_timeout(timeout)
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}
