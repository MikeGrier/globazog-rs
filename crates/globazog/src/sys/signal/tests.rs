// Copyright (c) 2026 Mike Grier

use super::Signal;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn notify_then_wait_returns_and_consumes() {
    let s = Signal::new();
    s.notify();
    s.wait(); // already signaled, returns immediately
    assert!(!s.try_take()); // wait consumed the signal
}

#[test]
fn notifies_coalesce() {
    let s = Signal::new();
    s.notify();
    s.notify();
    s.notify();
    assert!(s.try_take()); // one signal present
    assert!(!s.try_take()); // coalesced to a single signal
}

#[test]
fn cross_thread_wakeup() {
    let s = Arc::new(Signal::new());
    let s2 = Arc::clone(&s);
    let waiter = thread::spawn(move || {
        s2.wait();
        42
    });
    thread::sleep(Duration::from_millis(20));
    s.notify();
    assert_eq!(waiter.join().unwrap(), 42);
}
