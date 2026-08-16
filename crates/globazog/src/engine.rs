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

use crate::builder::{FollowLinks, Query};
use crate::error::EntryError;
use crate::predicate::{EntryType, MetaMask, eval_all};
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
    /// Index into `Query::roots` this scan belongs to; scopes which patterns apply
    /// (D-38). Inherited by child scans.
    root: usize,
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
    /// Containers whose `ContainerEnter` has been emitted but whose `ContainerEnd`
    /// has not yet been — keyed to depth for bottom-up teardown ordering. Every
    /// entry here is closed before the terminal, so the 1:1 enter/end guarantee holds
    /// even for a subtree abandoned by cancellation (D-64).
    open: HashMap<ContainerId, u32>,
    /// File ids of reparse dirs already descended, for cycle detection (D-51).
    visited: HashSet<FileId>,
}

/// The running engine, shared across worker/coordinator/SQ threads.
struct Engine {
    query: Query,
    patterns: PatternSet,
    fetch_mask: crate::predicate::MetaMask,
    /// What each directory enumeration must fetch (D-62): stat-tier fields only when a
    /// predicate/result-shape needs them, a file identity only when a followed reparse
    /// point needs cycle detection.
    enum_plan: sys::EnumPlan,
    ring: Arc<CompletionRing>,
    shared: Mutex<Shared>,
    work_cv: Condvar,
    cancel: AtomicBool,
    /// Set when a fatal error stops the walk (D-71); makes the terminal `Failed`.
    fatal: AtomicBool,
    /// The causing error of a fatal termination, handed to the coordinator so it is
    /// emitted immediately before `Terminal{Failed}` (D-71). First fatal wins.
    fatal_error: Mutex<Option<CqError>>,
    finished: AtomicBool,
    ids: IdSpace,
}

/// True once we have a real filesystem identity to key cycle detection on (the
/// unknown sentinel is `{0,0}`; see [`FileId`]). A partial `(0, id)` is never emitted,
/// so testing either field non-zero is sufficient.
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
                root: parent.root,
                name: ContainerName::Entry(Name::from_code_points(&entry.name)),
            });
            sh.outstanding += 1;
        }
        self.work_cv.notify_one();
    }

    /// Scan one directory: enter, enumerate, emit matches, launch descendants, and
    /// finalize the refcount (emitting any resulting ends).
    fn process(&self, job: ScanJob) {
        let depth = job.rel.len() as u32;

        // Open/enumerate BEFORE announcing the container: `ContainerEnter` is a
        // handle-open event (D-64), so a directory that cannot be opened must not
        // produce a phantom enter/end pair — it yields only an error (D-53), fatal
        // for a root (D-71).
        let scan = match sys::enumerate(&job.dir, self.enum_plan) {
            Ok(scan) => scan,
            Err(err) => {
                // No container was announced, so attribute the error to the parent
                // being scanned (`None` for a root) and name the directory that
                // failed to open where we have it (a child carries its own name; a
                // root carries only its index).
                let name = match &job.name {
                    ContainerName::Entry(n) => Some(n.clone()),
                    ContainerName::Root(_) => None,
                };
                let cq_err = CqError {
                    container: job.parent,
                    root: Some(job.root as u32),
                    error: EntryError { name, source: err },
                };
                if job.parent.is_none() {
                    // A root that cannot be enumerated is fatal (D-71). No container
                    // was entered, so just release accounting and hand the error to
                    // the coordinator to land immediately before Terminal{Failed}.
                    self.emit_ends(self.release(job.container));
                    self.request_fatal(cq_err);
                } else {
                    // Below the root: a per-directory failure is surfaced and the
                    // walk continues (D-53).
                    self.emit(CqItem::Error(cq_err));
                    self.emit_ends(self.release(job.container));
                }
                return;
            }
        };

        if !self.emit(CqItem::ContainerEnter(ContainerEnter {
            id: job.container,
            parent: job.parent,
            name: job.name.clone(),
        })) {
            // Cancelled before this enter was emitted: no enter, so no end is owed
            // for this container — but release its accounting so already-entered
            // ancestors that reach zero still get their (mandatory) ends.
            self.emit_ends(self.release(job.container));
            return;
        }
        // The enter is out; this container now owes an end, emitted no later than the
        // coordinator's teardown (even under cancellation).
        self.shared
            .lock()
            .unwrap()
            .open
            .insert(job.container, depth);
        // Surface each per-entry metadata failure as its own error item; the walk
        // continues with the entries that were read successfully (D-53).
        for failure in scan.entry_errors {
            self.emit(CqItem::Error(CqError {
                container: Some(job.container),
                root: Some(job.root as u32),
                error: EntryError {
                    name: failure.name.map(|cp| Name::from_code_points(&cp)),
                    source: failure.source,
                },
            }));
        }
        let entries = scan.entries;

        // Reused across entries: the parent's relative slices plus one reserved slot
        // for the current entry name. Each iteration only pushes/pops the leaf, so the
        // depth-sized path vector is allocated once per directory, not per entry.
        let mut path: Vec<&[CodePoint]> = job.rel.iter().map(|s| s.as_slice()).collect();
        path.reserve_exact(1);

        for entry in &entries {
            if self.cancel.load(Ordering::Acquire) {
                break;
            }
            let meta = entry.meta(depth);
            path.push(&entry.name);

            // Emit: per-pattern glob match AND that pattern's emit conjunction (D-66).
            // Only patterns that apply to this scan's root are considered, so an
            // anchored pattern never matches under an unrelated root (D-38).
            let mut hits = self.patterns.matches(&path);
            hits.retain(|&i| self.query.patterns[i].roots.contains(&job.root));
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
            // conjunction admits (D-56, D-66); reparse loops are cut (D-51). A
            // reparse point (symlink / junction) is a descend candidate only when the
            // follow policy opts in (D-72) — the library never auto-follows by
            // default (D-13); a followed non-directory target then fails to enumerate
            // and surfaces as a per-entry error (D-53).
            let dir_candidate = if entry.is_reparse {
                self.query.options.follow_links == FollowLinks::Always
            } else {
                entry.entry_type == EntryType::Dir
            };
            if dir_candidate
                && self.patterns.should_descend_where(&path, |i| {
                    self.query.patterns[i].roots.contains(&job.root)
                })
                && eval_all(&self.query.descend, &meta)
                && self.admit_descend(entry)
            {
                self.launch_child(&job, entry);
            }

            path.pop();
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

    /// Push a mandatory item (a `ContainerEnd` / fatal error / terminal,
    /// D-64/D-71), parking on a full ring but **never** bailing on cancel — the item
    /// must always land. Uses the same timed recheck loop as [`emit`](Self::emit)
    /// rather than the ring's single-producer `push_blocking`, so a `space` wake
    /// coalesced away between concurrent workers cannot strand one of them: the
    /// periodic recheck recovers a lost wake.
    fn push_mandatory(&self, item: CqItem) {
        let mut item = item;
        loop {
            match self.ring.try_push(item) {
                Ok(()) => return,
                Err(back) => {
                    item = back;
                    self.ring.wait_space_timeout(Duration::from_millis(5));
                }
            }
        }
    }

    /// Emit `id`'s `ContainerEnd` exactly once and cancel-immune (D-64): the end is
    /// mandatory, so it uses [`push_mandatory`](Self::push_mandatory) and is guarded
    /// by the `open` set, which makes it idempotent — a worker and the coordinator can
    /// never double-emit it.
    fn emit_end(&self, id: ContainerId) {
        let owed = self.shared.lock().unwrap().open.remove(&id).is_some();
        if owed {
            self.push_mandatory(CqItem::ContainerEnd(ContainerEnd { id }));
        }
    }

    fn emit_ends(&self, ends: Vec<ContainerId>) {
        for id in ends {
            self.emit_end(id);
        }
    }

    /// Trigger a fatal termination (D-71): stash the causing error for the
    /// coordinator to emit immediately before `Terminal{Failed}`, then stop the walk
    /// exactly like a cancel. First fatal wins; the error is not emitted here.
    fn request_fatal(&self, err: CqError) {
        {
            let mut slot = self.fatal_error.lock().unwrap();
            if slot.is_none() {
                *slot = Some(err);
            }
        }
        self.fatal.store(true, Ordering::Release);
        self.request_cancel();
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
    // The stat-tier fields require an actual stat; TYPE/REPARSE come from the listing.
    let stat_mask = MetaMask::SIZE
        | MetaMask::MTIME
        | MetaMask::ATIME
        | MetaMask::CTIME
        | MetaMask::BTIME
        | MetaMask::ATTRS;
    let enum_plan = sys::EnumPlan {
        want_stat: fetch_mask.intersects(stat_mask),
        // Cycle detection (D-51) only ever inspects a reparse point's id, so request
        // the narrow reparse-only identity rather than statting every entry.
        want_file_id: false,
        want_reparse_file_id: query.options.cycle_detection
            && query.options.follow_links == FollowLinks::Always,
    };
    let permits = query.options.permits.max(1);
    let ids = IdSpace::new();

    let mut shared = Shared {
        stack: Vec::new(),
        outstanding: 0,
        cancelled: false,
        containers: HashMap::new(),
        open: HashMap::new(),
        visited: HashSet::new(),
    };
    // Seed only roots that some pattern actually applies to (D-38): a supplied root
    // with no applicable pattern (e.g. an anchored-only query) is never walked, so an
    // unrelated bad supplied root cannot fail an otherwise-valid anchored traversal.
    let mut referenced = vec![false; query.roots.len()];
    for p in &query.patterns {
        for &r in &p.roots {
            referenced[r] = true;
        }
    }
    for (i, root) in query.roots.iter().enumerate() {
        if !referenced[i] {
            continue;
        }
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
            root: i,
            name: ContainerName::Root(i as u32),
        });
    }

    let engine = Arc::new(Engine {
        query,
        patterns,
        fetch_mask,
        enum_plan,
        ring,
        shared: Mutex::new(shared),
        work_cv: Condvar::new(),
        cancel: AtomicBool::new(false),
        fatal: AtomicBool::new(false),
        fatal_error: Mutex::new(None),
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
            // Close any container still open (e.g. a subtree abandoned by
            // cancellation): the 1:1 ContainerEnter/ContainerEnd guarantee must hold
            // before the terminal (D-64). Deepest-first gives bottom-up ends.
            let mut leftovers: Vec<(ContainerId, u32)> = {
                let sh = e.shared.lock().unwrap();
                sh.open.iter().map(|(&id, &d)| (id, d)).collect()
            };
            leftovers.sort_by_key(|&(_, depth)| std::cmp::Reverse(depth));
            for (id, _) in leftovers {
                e.emit_end(id);
            }
            let reason = if e.fatal.load(Ordering::Acquire) {
                TerminalReason::Failed
            } else if e.cancel.load(Ordering::Acquire) {
                TerminalReason::Cancelled
            } else {
                TerminalReason::Completed
            };
            // On a fatal termination emit the causing error last, immediately before
            // the terminal, so the D-71 ordering contract holds regardless of what
            // the workers enqueued (all have joined by now).
            if let Some(err) = e.fatal_error.lock().unwrap().take() {
                e.push_mandatory(CqItem::Error(err));
            }
            // Terminal lands after all queued items (FIFO, D-61); block until the
            // client (or the drop-drain) makes room.
            e.push_mandatory(CqItem::Terminal(Terminal { reason }));
            e.finished.store(true, Ordering::Release);
        })
    };

    EngineHandle {
        engine,
        coordinator: Some(coordinator),
        sq_thread: Some(sq_thread),
    }
}
