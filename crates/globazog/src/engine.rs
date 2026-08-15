// Copyright (c) 2026 Mike Grier

//! The Model B scheduler (D-3): permit-bounded directory scans (D-7, D-8), the
//! parent-handle refcount cascade that drives `ContainerEnd` (D-9, D-10, D-64),
//! depth-first enumerate-then-recurse (D-48), the unifying continuation suspension
//! (D-59) — realized in this synchronous backend as worker-thread blocking for both
//! I/O wait (D-6) and output backpressure (D-11) — cancellation accounting (D-50,
//! D-61), and reparse-cycle detection (D-51).
//!
//! This is the functional engine over the **synchronous** enumeration backends
//! (native where available, [`sys::enumerate`]). The Windows overlapped-IOCP /
//! Linux io_uring async orchestration (M7-6) layers behind the same contract later;
//! the `defer-to-client` predicate escalation (D-58) is a forward item — it needs a
//! tri-state predicate leaf that the M4 vocabulary does not yet have.

use crate::builder::Query;
use crate::error::EntryError;
use crate::predicate::{EntryType, eval_all};
use crate::ring::{
    CompletionRing, ContainerEnd, ContainerEnter, ContainerId, ContainerName, CqError, CqItem,
    EntryMetaOwned, IdSpace, Match, Name, PatternMask, SqOp, SubmissionQueue, Terminal,
    TerminalReason,
};
use crate::syntax::CodePoint;
use crate::syntax::set::PatternSet;
use crate::sys::{self, DirEntry, FileId};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One unit of work: scan a single directory (D-7).
struct ScanJob {
    /// Physical path to enumerate.
    dir: PathBuf,
    /// This directory's path segments from its root (drives matching + depth).
    rel: Arc<Vec<Vec<CodePoint>>>,
    /// This directory's container id.
    container: ContainerId,
    /// Parent container, or `None` for a root.
    parent: Option<ContainerId>,
    /// The `ContainerEnter` name (entry blob, or root index for a root).
    name: ContainerName,
}

/// Per-container accounting: its parent and a refcount = 1 (own scan) + outstanding
/// children (D-9). `ContainerEnd` fires when it reaches zero, cascading up (D-64).
struct ContainerState {
    parent: Option<ContainerId>,
    refcount: usize,
}

/// Engine state guarded by one mutex; workers coordinate on `work_cv`.
struct Shared {
    /// Pending scan jobs (LIFO for depth-first bias, D-48).
    stack: Vec<ScanJob>,
    /// Jobs created but not yet fully processed; 0 means the walk is complete.
    outstanding: usize,
    /// Set once cancellation is requested (D-61).
    cancelled: bool,
    /// Live container accounting.
    containers: HashMap<ContainerId, ContainerState>,
    /// File ids of reparse dirs already descended, for cycle detection (D-51).
    visited: HashSet<FileId>,
}

/// The running engine, shared across worker/coordinator/SQ threads.
struct Engine {
    query: Query,
    patterns: PatternSet,
    fetch_mask: crate::predicate::MetaMask,
    ring: Arc<CompletionRing>,
    shared: Mutex<Shared>,
    work_cv: Condvar,
    cancel: AtomicBool,
    finished: AtomicBool,
    ids: IdSpace,
}

/// True once we have a real filesystem identity (portable Windows leaves it zero).
fn file_id_known(id: FileId) -> bool {
    id.volume != 0 || id.id != 0
}

impl Engine {
    /// Push a CQ item, parking on a full ring (D-11) but bailing out promptly on
    /// cancellation so teardown never deadlocks. Returns whether the item was
    /// enqueued.
    fn emit(&self, item: CqItem) -> bool {
        let mut item = item;
        loop {
            if self.cancel.load(Ordering::Acquire) {
                return false;
            }
            match self.ring.try_push(item) {
                Ok(()) => return true,
                Err(back) => {
                    item = back;
                    self.ring.wait_space_timeout(Duration::from_millis(5));
                }
            }
        }
    }

    /// Take the next job, blocking while work is in flight; `None` means this worker
    /// should exit (all work drained, or cancelled).
    fn next_job(&self) -> Option<ScanJob> {
        let mut sh = self.shared.lock().unwrap();
        loop {
            if sh.cancelled {
                return None;
            }
            if let Some(job) = sh.stack.pop() {
                return Some(job);
            }
            if sh.outstanding == 0 {
                self.work_cv.notify_all();
                return None;
            }
            sh = self.work_cv.wait(sh).unwrap();
        }
    }

    /// Mark a job complete; wake idle workers to exit when the last one finishes.
    fn complete_job(&self) {
        let mut sh = self.shared.lock().unwrap();
        sh.outstanding -= 1;
        if sh.outstanding == 0 {
            self.work_cv.notify_all();
        }
    }

    /// Decrement `id`'s refcount and cascade upward, returning the containers that
    /// reached zero (bottom-up) so their `ContainerEnd`s can be emitted outside the
    /// lock (D-64).
    fn release(&self, start: ContainerId) -> Vec<ContainerId> {
        let mut ends = Vec::new();
        let mut sh = self.shared.lock().unwrap();
        let mut id = start;
        loop {
            let st = sh
                .containers
                .get_mut(&id)
                .expect("container state present until released");
            st.refcount -= 1;
            if st.refcount == 0 {
                let parent = st.parent;
                sh.containers.remove(&id);
                ends.push(id);
                match parent {
                    Some(p) => id = p,
                    None => break,
                }
            } else {
                break;
            }
        }
        ends
    }

    /// Register a child container under `parent` and enqueue its scan (D-9, D-10).
    fn launch_child(&self, parent: &ScanJob, entry: &DirEntry) {
        let child_id = self.ids.next_container();
        let mut child_rel = (*parent.rel).clone();
        child_rel.push(entry.name.clone());
        let child_dir = parent.dir.join(sys::encode_os_name(&entry.name));

        {
            let mut sh = self.shared.lock().unwrap();
            sh.containers.insert(
                child_id,
                ContainerState {
                    parent: Some(parent.container),
                    refcount: 1,
                },
            );
            if let Some(st) = sh.containers.get_mut(&parent.container) {
                st.refcount += 1;
            }
            sh.stack.push(ScanJob {
                dir: child_dir,
                rel: Arc::new(child_rel),
                container: child_id,
                parent: Some(parent.container),
                name: ContainerName::Entry(Name::from_code_points(&entry.name)),
            });
            sh.outstanding += 1;
        }
        self.work_cv.notify_one();
    }

    /// Scan one directory: enter, enumerate, emit matches, launch descendants, and
    /// finalize the refcount (emitting any resulting ends).
    fn process(&self, job: ScanJob) {
        if !self.emit(CqItem::ContainerEnter(ContainerEnter {
            id: job.container,
            parent: job.parent,
            name: job.name.clone(),
        })) {
            // Cancelled during teardown: still release accounting; skip end emits.
            let _ = self.release(job.container);
            return;
        }

        let scan = match sys::enumerate(&job.dir) {
            Ok(scan) => scan,
            Err(err) => {
                self.emit(CqItem::Error(CqError {
                    container: Some(job.container),
                    error: EntryError { source: err },
                }));
                self.emit_ends(self.release(job.container));
                return;
            }
        };
        // Per-entry failures (M9-2) and the fatal-root terminal (M9-3) consume
        // `scan.entry_errors`; for now the readable entries drive the walk.
        let entries = scan.entries;

        let rel_slices: Vec<&[CodePoint]> = job.rel.iter().map(|s| s.as_slice()).collect();
        let depth = job.rel.len() as u32;

        for entry in &entries {
            if self.cancel.load(Ordering::Acquire) {
                break;
            }
            let meta = entry.meta(depth);
            let mut path = rel_slices.clone();
            path.push(&entry.name);

            // Emit: per-pattern glob match AND that pattern's emit conjunction (D-66).
            let hits = self.patterns.matches(&path);
            if !hits.is_empty() {
                let mut mask = PatternMask::new(self.query.patterns.len());
                let mut any = false;
                for i in hits {
                    if eval_all(&self.query.patterns[i].emit, &meta) {
                        mask.set(i);
                        any = true;
                    }
                }
                if any {
                    self.emit(CqItem::Match(Match {
                        container: job.container,
                        name: Name::from_code_points(&entry.name),
                        matched: mask,
                        meta: EntryMetaOwned::from_meta(&meta, self.fetch_mask),
                    }));
                }
            }

            // Descend: a directory that some pattern still wants AND the descend
            // conjunction admits (D-56, D-66); reparse loops are cut (D-51).
            if entry.entry_type == EntryType::Dir
                && self.patterns.should_descend(&path)
                && eval_all(&self.query.descend, &meta)
                && self.admit_descend(entry)
            {
                self.launch_child(&job, entry);
            }
        }

        self.emit_ends(self.release(job.container));
    }

    /// Cycle-detection gate (D-51): a reparse-point directory is descended only once
    /// per filesystem identity. Non-reparse dirs and unknown ids pass through.
    fn admit_descend(&self, entry: &DirEntry) -> bool {
        if !(entry.is_reparse && self.query.options.cycle_detection && file_id_known(entry.file_id))
        {
            return true;
        }
        let mut sh = self.shared.lock().unwrap();
        sh.visited.insert(entry.file_id)
    }

    fn emit_ends(&self, ends: Vec<ContainerId>) {
        for id in ends {
            self.emit(CqItem::ContainerEnd(ContainerEnd { id }));
        }
    }

    /// Request cancellation (idempotent): wake workers and any parked emitters.
    fn request_cancel(&self) {
        if self.cancel.swap(true, Ordering::AcqRel) {
            return;
        }
        {
            let mut sh = self.shared.lock().unwrap();
            sh.cancelled = true;
        }
        self.work_cv.notify_all();
        self.ring.wake_producers();
    }
}

fn worker_loop(engine: &Arc<Engine>) {
    while let Some(job) = engine.next_job() {
        engine.process(job);
        engine.complete_job();
    }
}

fn sq_loop(engine: &Arc<Engine>, sq: &Arc<SubmissionQueue>) {
    while !engine.finished.load(Ordering::Acquire) {
        for op in sq.drain() {
            match op {
                SqOp::Cancel => engine.request_cancel(),
                // `defer-to-client` answers have no waiter yet (forward item).
                SqOp::DecisionAnswer { .. } => {}
                // The engine is already running; a re-submit is a no-op here.
                SqOp::SubmitQuery(_) => {}
            }
        }
        sq.wait_timeout(Duration::from_millis(25));
    }
}

/// A running query's engine threads. Dropping it cancels and joins (D-61).
/// Cancellation during the run flows through the SQ (`SqOp::Cancel`, D-61); this
/// handle's `Drop` is the RAII teardown join.
pub struct EngineHandle {
    engine: Arc<Engine>,
    coordinator: Option<JoinHandle<()>>,
    sq_thread: Option<JoinHandle<()>>,
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.engine.request_cancel();
        // Drain so a coordinator/worker parked on a full ring can finish, avoiding a
        // teardown deadlock if the client stopped draining (D-61).
        while !self.engine.finished.load(Ordering::Acquire) {
            while self.engine.ring.pop().is_some() {}
            std::thread::yield_now();
        }
        while self.engine.ring.pop().is_some() {}
        if let Some(h) = self.coordinator.take() {
            let _ = h.join();
        }
        if let Some(h) = self.sq_thread.take() {
            let _ = h.join();
        }
    }
}

/// Spawn the engine for `query`, populating `ring` in the background (D-3). Returns
/// immediately; the client services the ring concurrently.
pub fn spawn(query: Query, ring: Arc<CompletionRing>, sq: Arc<SubmissionQueue>) -> EngineHandle {
    let patterns =
        PatternSet::from_compiled(query.patterns.iter().map(|p| p.glob.clone()).collect());
    let fetch_mask = query.fetch_mask();
    let permits = query.options.permits.max(1);
    let ids = IdSpace::new();

    let mut shared = Shared {
        stack: Vec::new(),
        outstanding: 0,
        cancelled: false,
        containers: HashMap::new(),
        visited: HashSet::new(),
    };
    for (i, root) in query.roots.iter().enumerate() {
        let cid = ids.next_container();
        shared.containers.insert(
            cid,
            ContainerState {
                parent: None,
                refcount: 1,
            },
        );
        shared.outstanding += 1;
        shared.stack.push(ScanJob {
            dir: root.path.clone(),
            rel: Arc::new(Vec::new()),
            container: cid,
            parent: None,
            name: ContainerName::Root(i as u32),
        });
    }

    let engine = Arc::new(Engine {
        query,
        patterns,
        fetch_mask,
        ring,
        shared: Mutex::new(shared),
        work_cv: Condvar::new(),
        cancel: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        ids,
    });

    let mut workers = Vec::with_capacity(permits);
    for _ in 0..permits {
        let e = Arc::clone(&engine);
        workers.push(std::thread::spawn(move || worker_loop(&e)));
    }

    let sq_thread = {
        let e = Arc::clone(&engine);
        let sq = Arc::clone(&sq);
        std::thread::spawn(move || sq_loop(&e, &sq))
    };

    let coordinator = {
        let e = Arc::clone(&engine);
        std::thread::spawn(move || {
            for w in workers {
                let _ = w.join();
            }
            let reason = if e.cancel.load(Ordering::Acquire) {
                TerminalReason::Cancelled
            } else {
                TerminalReason::Completed
            };
            // Terminal lands after all queued items (FIFO, D-61); block until the
            // client (or the drop-drain) makes room.
            e.ring.push_blocking(CqItem::Terminal(Terminal { reason }));
            e.finished.store(true, Ordering::Release);
        })
    };

    EngineHandle {
        engine,
        coordinator: Some(coordinator),
        sq_thread: Some(sq_thread),
    }
}
