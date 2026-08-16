# globazog — Build Checklist

Action queue only — pending and in-progress work. Completed milestones are archived in
[COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md) (status index in [PLANS.md](PLANS.md));
the design rationale is in [DESIGN-NOTES.md](DESIGN-NOTES.md).

## M7+ — Engine follow-ups (parked, gated beyond completed M7)

These are **parked, not pending**: milestone M7 is complete and archived in
[COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md). The two items below are gated on
deliverables that do not yet exist (the native async backends / the D-5 completion
abstraction, and the tri-state predicate refactor), so they sit in the `M7+`
placeholder bucket and keep their stable historical IDs (`M7-6`, `M7-7`,
cross-referenced from the source and design notes). They graduate to a real numbered
milestone when that later work is authored — they are **not** open obligations of the
completed M7.

- [ ] **M7-6. Native async enumeration backends + relative-open** (D-4, D-6, D-9,
  D-59): overlapped Windows enumeration + IOCP + `CreateThreadpoolIo` (`TP_IO`) +
  `TP_WORK`, and the Linux io_uring path where available, driven by the M7 unified
  park/resume continuation (D-59); as part of this, thread a **parent directory
  handle** through the enumeration backend so children open relative to it (openat /
  handle-relative `NtCreateFile`, D-9), replacing the current full-path open. Restore
  the `\\?\` NT-layer open (dropped when M5-4 switched to `std::fs::OpenOptions`) so
  Windows **long paths (>260)** and **trailing-dot-space names** work — and add the
  M8-2 integration coverage for them (those files cannot be created through the Win32
  layer, so the tests belong here). Re-introduce the SQ **query-submission (boot) op**
  here — it was removed from the synchronous engine as inert (the sync engine boots
  directly via `QueryBuilder::submit`); the reactor model is where an SQ-driven submit
  becomes meaningful (D-66). **Gated on** building the D-5 completion abstraction + the
  native async FFI, not a missing consumer.

- [ ] **M7-7. `defer-to-client` predicate escalation** (D-58): add a **tri-state**
  predicate leaf (accept / reject / defer) to the M4 vocabulary; on defer, emit a
  `DecisionRequest` (already a CQ variant), park the scan on the unified suspension,
  and resume when the client answers via `SqOp::DecisionAnswer` (already SQ-wired).
  **Gated on** the tri-state predicate refactor (M4 currently returns bool), not a
  missing consumer.

## M∞ — Horizon (gated on profiling)

- [ ] **M∞-1. Zero-copy inline name blob + over-cap spill** (D-63, D-68, D-69):
  replace the owned code-point `Vec` name with a fixed inline native-unit buffer in
  a 512-byte descriptor slot, add the borrowed / lending-cursor read path, and
  implement the pathological over-cap spill (spill-to-side or error item).
  **Gated on** profiling showing the owned-copy cost matters (D-68/D-69), not on a
  missing consumer.

- [ ] **M∞-2. Syscall-level FS-filter pushdown** (D-19, D-70): at a terminal literal
  pattern segment, push a name filter into the enumeration syscall **where the API
  accepts one** — the Windows `FindFirstFileW` wildcard — instead of enumerating the
  whole directory and matching in-process. Linux `getdents64` / `readdir` has **no**
  kernel-side name or prefix filter, so it **retains in-process filtering** (no
  pushdown there). Sound because the pattern is gospel (D-19); the
  in-process descend pruning already bounds *which directories* are scanned, so this
  is a pure per-directory perf add. **Gated on** profiling, not a missing consumer.

- [ ] **M∞-3. Multi-frame overlapping-root merge** (full D-37): enumerate a directory
  reachable from several supplied roots **once**, but emit its matches under **every**
  applicable `(root, root-relative)` frame — the alternative to the current D-73
  reject-overlap policy. **Gated on** deciding the multi-frame directory identity model
  is wanted over rejecting overlap (a public output-semantics change), not a missing
  consumer.
