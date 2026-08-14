// Copyright (c) 2026 Mike Grier

//! M6-5 integration test: the completion-ring contract (D-64, D-68, D-61). These
//! tests drive the ring the way the engine (M7) will — many producers, one
//! consumer — and assert the ordering invariants, bounded no-drop backpressure, and
//! cancellation terminal-marker semantics without any real filesystem traversal.

use globazog::predicate::{EntryType, MetaMask};
use globazog::ring::{
    CompletionRing, ContainerEnd, ContainerEnter, ContainerId, ContainerName, CqItem,
    EntryMetaOwned, IdSpace, Match, Name, PatternMask, Terminal, TerminalReason,
};
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::thread;

fn cid(n: u64) -> ContainerId {
    ContainerId(NonZeroU64::new(n).unwrap())
}

fn meta(depth: u32) -> EntryMetaOwned {
    EntryMetaOwned {
        present: MetaMask::TYPE,
        depth,
        entry_type: EntryType::File,
        is_reparse: false,
        reparse_tag: 0,
        attributes: 0,
        size: 0,
        btime: 0,
        mtime: 0,
        atime: 0,
        ctime: 0,
    }
}

/// Build a synthetic tree of `fanout` top-level dirs, each with `depth` nested dirs,
/// and drive the events through the ring in the exact order the engine promises:
/// a `ContainerEnter` precedes any use of its id as a parent, and `ContainerEnd`s
/// cascade bottom-up. The consumer validates both invariants.
#[test]
fn container_enter_before_children_and_ends_cascade_bottom_up() {
    let fanout = 20u64;
    let depth = 5u64;
    let ring = Arc::new(CompletionRing::with_capacity(16));
    let ids = IdSpace::new();

    // Producer: emit a valid enter/end stream. Root children are chains of depth
    // `depth`; ends are emitted deepest-first.
    let producer = {
        let ring = Arc::clone(&ring);
        // Pre-allocate ids so the producer thread emits a deterministic stream.
        let mut chains: Vec<Vec<ContainerId>> = Vec::new();
        for _ in 0..fanout {
            let mut chain = Vec::new();
            for _ in 0..depth {
                chain.push(ids.next_container());
            }
            chains.push(chain);
        }
        thread::spawn(move || {
            for chain in &chains {
                // Enter each level top-down; parent is the previous level (root
                // sentinel for the first).
                let mut parent: Option<ContainerId> = None;
                for (level, &id) in chain.iter().enumerate() {
                    ring.push_blocking(CqItem::ContainerEnter(ContainerEnter {
                        id,
                        parent,
                        name: match parent {
                            Some(_) => ContainerName::Entry(Name::from_code_points(&[
                                b'l' as u32,
                                b'0' as u32 + level as u32,
                            ])),
                            None => ContainerName::Root(0),
                        },
                    }));
                    // A discovered match inside this container.
                    ring.push_blocking(CqItem::Match(Match {
                        container: id,
                        name: Name::from_code_points(&[b'f' as u32]),
                        matched: PatternMask::from_indices(1, &[0]),
                        meta: meta(level as u32),
                    }));
                    parent = Some(id);
                }
                // Ends cascade bottom-up (deepest first).
                for &id in chain.iter().rev() {
                    ring.push_blocking(CqItem::ContainerEnd(ContainerEnd { id }));
                }
            }
            ring.push_blocking(CqItem::Terminal(Terminal {
                reason: TerminalReason::Completed,
            }));
        })
    };

    let consumer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            let mut live: HashMap<ContainerId, Option<ContainerId>> = HashMap::new();
            let mut ended: std::collections::HashSet<ContainerId> =
                std::collections::HashSet::new();
            let mut enters = 0u64;
            let mut ends = 0u64;
            let mut matches = 0u64;
            loop {
                match ring.wait_pop() {
                    CqItem::ContainerEnter(e) => {
                        // Invariant 1: the parent must have been announced already
                        // and not yet ended.
                        if let Some(parent) = e.parent {
                            assert!(
                                live.contains_key(&parent),
                                "child announced before its parent's enter"
                            );
                            assert!(
                                !ended.contains(&parent),
                                "child announced after parent ended"
                            );
                        }
                        assert!(live.insert(e.id, e.parent).is_none(), "duplicate enter");
                        enters += 1;
                    }
                    CqItem::Match(m) => {
                        assert!(
                            live.contains_key(&m.container),
                            "match in unknown container"
                        );
                        assert!(!ended.contains(&m.container), "match after container ended");
                        assert!(m.matched.contains(0));
                        matches += 1;
                    }
                    CqItem::ContainerEnd(end) => {
                        let parent = live.remove(&end.id).expect("end without live enter");
                        // Invariant 2: bottom-up cascade — a container ends only
                        // after all its children have ended (none remain live).
                        assert!(
                            !live.values().any(|p| *p == Some(end.id)),
                            "container ended while a child was still live"
                        );
                        if let Some(p) = parent {
                            assert!(!ended.contains(&p), "parent ended before child");
                        }
                        ended.insert(end.id);
                        ends += 1;
                    }
                    CqItem::Terminal(t) => {
                        assert_eq!(t.reason, TerminalReason::Completed);
                        // Everything closed by the terminal marker.
                        assert!(live.is_empty(), "containers still live at terminal");
                        return (enters, ends, matches);
                    }
                    other => panic!("unexpected item {other:?}"),
                }
            }
        })
    };

    producer.join().unwrap();
    let (enters, ends, matches) = consumer.join().unwrap();
    assert_eq!(enters, fanout * depth);
    assert_eq!(ends, fanout * depth);
    assert_eq!(matches, fanout * depth);
}

/// Bounded, no-drop backpressure (D-11, D-68): the producer emits far more items
/// than the tiny ring can hold; every item is delivered exactly once, in order.
#[test]
fn bounded_ring_delivers_every_item_under_backpressure() {
    let ring = Arc::new(CompletionRing::with_capacity(2));
    let total = 2000u64;

    let producer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            for i in 1..=total {
                ring.push_blocking(CqItem::ContainerEnd(ContainerEnd { id: cid(i) }));
            }
        })
    };

    let consumer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            let mut next = 1u64;
            while next <= total {
                if let CqItem::ContainerEnd(e) = ring.wait_pop() {
                    assert_eq!(e.id.0.get(), next, "delivery must be FIFO with no gaps");
                    next += 1;
                }
            }
            next - 1
        })
    };

    producer.join().unwrap();
    let delivered = consumer.join().unwrap();
    assert_eq!(delivered, total);
}

/// Cancellation ordering (D-61): a terminal marker enqueued after in-flight items
/// is delivered strictly last (drain-what-is-queued-then-terminal).
#[test]
fn cancellation_terminal_marker_is_last() {
    let ring = CompletionRing::with_capacity(64);
    for i in 1..=10 {
        ring.try_push(CqItem::ContainerEnd(ContainerEnd { id: cid(i) }))
            .unwrap();
    }
    ring.try_push(CqItem::Terminal(Terminal {
        reason: TerminalReason::Cancelled,
    }))
    .unwrap();

    let items = ring.drain();
    assert_eq!(items.len(), 11);
    for (i, item) in items.iter().take(10).enumerate() {
        match item {
            CqItem::ContainerEnd(e) => assert_eq!(e.id.0.get() as usize, i + 1),
            other => panic!("expected end, got {other:?}"),
        }
    }
    match items.last().unwrap() {
        CqItem::Terminal(t) => assert_eq!(t.reason, TerminalReason::Cancelled),
        other => panic!("terminal must be last, got {other:?}"),
    }
}
