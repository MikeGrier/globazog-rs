// Copyright (c) 2026 Mike Grier

use super::*;
use crate::predicate::EntryType;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::thread;

fn cid(n: u64) -> ContainerId {
    ContainerId(NonZeroU64::new(n).unwrap())
}

fn enter(id: u64, parent: Option<u64>) -> CqItem {
    CqItem::ContainerEnter(ContainerEnter {
        id: cid(id),
        parent: parent.map(cid),
        name: match parent {
            Some(_) => ContainerName::Entry(Name::from_code_points(&[b'd' as u32])),
            None => ContainerName::Root(0),
        },
    })
}

fn end(id: u64) -> CqItem {
    CqItem::ContainerEnd(ContainerEnd { id: cid(id) })
}

#[test]
fn id_space_is_monotonic_and_nonzero() {
    let space = IdSpace::new();
    let a = space.next_container();
    let b = space.next_container();
    let c = space.next_decision();
    assert_eq!(a.0.get(), 1);
    assert_eq!(b.0.get(), 2);
    assert_eq!(c.0.get(), 3);
    assert_ne!(a, b);
}

#[test]
fn pattern_mask_set_contains_iter() {
    let mut m = PatternMask::new(130);
    assert!(!m.any());
    m.set(0);
    m.set(63);
    m.set(64);
    m.set(129);
    assert!(m.contains(0));
    assert!(m.contains(63));
    assert!(m.contains(64));
    assert!(m.contains(129));
    assert!(!m.contains(1));
    assert!(!m.contains(128));
    assert!(m.any());
    assert_eq!(m.count(), 4);
    assert_eq!(m.iter().collect::<Vec<_>>(), vec![0, 63, 64, 129]);
}

#[test]
fn pattern_mask_from_indices() {
    let m = PatternMask::from_indices(8, &[1, 3, 5]);
    assert_eq!(m.iter().collect::<Vec<_>>(), vec![1, 3, 5]);
    assert!(!m.contains(0));
}

#[test]
fn name_round_trips_and_lossy_string() {
    let cps = [b'h' as u32, b'i' as u32];
    let n = Name::from_code_points(&cps);
    assert_eq!(n.code_points(), &cps);
    assert_eq!(n.len(), 2);
    assert!(!n.is_empty());
    assert_eq!(n.to_string_lossy(), "hi");
}

#[test]
fn name_lossy_maps_invalid_to_replacement() {
    // A lone surrogate code point is not a scalar value.
    let n = Name::from_code_points(&[0xD800]);
    assert_eq!(n.to_string_lossy(), "\u{FFFD}");
}

#[test]
fn entry_meta_owned_borrow_round_trip() {
    let owned = EntryMetaOwned {
        present: MetaMask::SIZE | MetaMask::TYPE,
        depth: 3,
        entry_type: EntryType::File,
        is_reparse: false,
        reparse_tag: 0,
        attributes: 0,
        size: 42,
        btime: 0,
        mtime: 0,
        atime: 0,
        ctime: 0,
    };
    let name = [b'x' as u32];
    let borrowed = owned.borrow(&name);
    assert_eq!(borrowed.name, &name);
    assert_eq!(borrowed.depth, 3);
    assert_eq!(borrowed.size, 42);
    assert_eq!(borrowed.entry_type, EntryType::File);
}

#[test]
fn ring_push_pop_fifo() {
    let ring = CompletionRing::with_capacity(4);
    assert!(ring.is_empty());
    ring.try_push(enter(1, None)).unwrap();
    ring.try_push(enter(2, Some(1))).unwrap();
    assert_eq!(ring.len(), 2);

    match ring.pop().unwrap() {
        CqItem::ContainerEnter(e) => assert_eq!(e.id, cid(1)),
        other => panic!("expected enter(1), got {other:?}"),
    }
    match ring.pop().unwrap() {
        CqItem::ContainerEnter(e) => assert_eq!(e.id, cid(2)),
        other => panic!("expected enter(2), got {other:?}"),
    }
    assert!(ring.pop().is_none());
}

#[test]
fn ring_full_returns_item_back() {
    let ring = CompletionRing::with_capacity(2);
    ring.try_push(end(1)).unwrap();
    ring.try_push(end(2)).unwrap();
    assert!(ring.is_full());
    let back = ring.try_push(end(3));
    assert!(back.is_err());
    // The rejected item is handed back intact.
    match back.unwrap_err() {
        CqItem::ContainerEnd(e) => assert_eq!(e.id, cid(3)),
        other => panic!("expected end(3), got {other:?}"),
    }
}

#[test]
fn ring_drain_empties() {
    let ring = CompletionRing::with_capacity(8);
    for i in 1..=5 {
        ring.try_push(end(i)).unwrap();
    }
    let drained = ring.drain();
    assert_eq!(drained.len(), 5);
    assert!(ring.is_empty());
    assert!(ring.drain().is_empty());
}

#[test]
fn ring_backpressure_no_drop_across_threads() {
    // A bounded ring with more items than capacity: the producer blocks on the
    // space signal; the consumer drains at its own rate; nothing is dropped.
    let ring = Arc::new(CompletionRing::with_capacity(4));
    let total = 500u64;

    let producer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            for i in 1..=total {
                ring.push_blocking(end(i));
            }
        })
    };

    let seen = Arc::new(AtomicUsize::new(0));
    let consumer = {
        let ring = Arc::clone(&ring);
        let seen = Arc::clone(&seen);
        thread::spawn(move || {
            let mut expected = 1u64;
            while (seen.load(AtomicOrdering::Relaxed) as u64) < total {
                let item = ring.wait_pop();
                match item {
                    CqItem::ContainerEnd(e) => {
                        assert_eq!(e.id.0.get(), expected, "items must arrive in FIFO order");
                        expected += 1;
                        seen.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                    other => panic!("unexpected item {other:?}"),
                }
            }
        })
    };

    producer.join().unwrap();
    consumer.join().unwrap();
    assert_eq!(seen.load(AtomicOrdering::Relaxed) as u64, total);
}

#[test]
fn submission_queue_fifo_and_drain() {
    let sq = SubmissionQueue::new();
    assert!(sq.is_empty());
    sq.submit(SqOp::Cancel);
    sq.submit(SqOp::DecisionAnswer {
        token: DecisionToken(NonZeroU64::new(7).unwrap()),
        decision: Decision::Accept,
    });
    assert!(!sq.is_empty());

    match sq.try_next().unwrap() {
        SqOp::Cancel => {}
        other => panic!("expected Cancel, got {other:?}"),
    }
    let rest = sq.drain();
    assert_eq!(rest.len(), 1);
    assert!(sq.is_empty());
}

#[test]
fn terminal_marker_lands_after_queued_items() {
    // Cancellation is drain-what-is-queued-then-terminal (D-61): the terminal
    // marker is enqueued after the items already present, so FIFO delivers it last.
    let ring = CompletionRing::with_capacity(8);
    ring.try_push(enter(1, None)).unwrap();
    ring.try_push(end(1)).unwrap();
    ring.try_push(CqItem::Terminal(Terminal {
        reason: TerminalReason::Cancelled,
    }))
    .unwrap();

    let items = ring.drain();
    assert_eq!(items.len(), 3);
    match items.last().unwrap() {
        CqItem::Terminal(t) => assert_eq!(t.reason, TerminalReason::Cancelled),
        other => panic!("terminal must be last, got {other:?}"),
    }
}
