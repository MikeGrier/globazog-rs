// Copyright (c) 2026 Mike Grier

//! A coalesced waitable primitive (D-60): signals ring readiness / backpressure
//! wakeups. Raw OS-handle exposure (Windows event / Linux eventfd) for foreign
//! reactors is a native follow-up; this portable `Condvar` form is what the engine
//! uses internally.

use std::sync::{Condvar, Mutex};

#[cfg(test)]
mod tests;

/// A coalesced binary signal. Multiple [`notify`](Signal::notify)s before a
/// [`wait`](Signal::wait) collapse into one (no signal storm), matching D-60's
/// empty→non-empty coalescing.
#[derive(Debug, Default)]
pub struct Signal {
    set: Mutex<bool>,
    cv: Condvar,
}

impl Signal {
    /// A new, unsignaled `Signal`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Raise the signal and wake one waiter.
    pub fn notify(&self) {
        let mut set = self.set.lock().unwrap();
        *set = true;
        self.cv.notify_one();
    }

    /// Block until signaled, then consume the signal.
    pub fn wait(&self) {
        let mut set = self.set.lock().unwrap();
        while !*set {
            set = self.cv.wait(set).unwrap();
        }
        *set = false;
    }

    /// Block until signaled or `timeout` elapses; returns whether it was signaled.
    /// Used for cancel-responsive backpressure waits (D-11): a parked producer
    /// re-checks the cancel flag each time the wait returns.
    ///
    /// `Condvar` may wake spuriously, so the wait is a predicate loop
    /// (`wait_timeout_while`) that only returns early on a real signal; a `false`
    /// result therefore means the full `timeout` elapsed, never a spurious wake.
    pub fn wait_timeout(&self, timeout: std::time::Duration) -> bool {
        let set = self.set.lock().unwrap();
        let (mut set, _res) = self
            .cv
            .wait_timeout_while(set, timeout, |signaled| !*signaled)
            .unwrap();
        if *set {
            *set = false;
            true
        } else {
            false
        }
    }

    /// Consume and return the current signal without blocking.
    pub fn try_take(&self) -> bool {
        let mut set = self.set.lock().unwrap();
        let was = *set;
        *set = false;
        was
    }
}
