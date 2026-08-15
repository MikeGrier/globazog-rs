# globazog — Build Checklist

Built **bottom-up** per [DESIGN-NOTES.md](DESIGN-NOTES.md). Completed milestones
**M1–M9** (matcher → dialects → predicates → `sys` incl. native Win/Linux backends →
ring → the synchronous Model B engine → end-to-end integration, example, docs →
per-entry error propagation + fatal-error terminal) are archived in
[COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md). Only the remaining pending work is
listed below.

End-of-milestone steps (repo standard, **not** listed as items): build the default
workspace debug+release with zero warnings; test the in-scope crate; sync with
origin and push.

## M7 — Engine (the scheduler)

- [ ] **M7-6. Native async enumeration backends + relative-open** (D-4, D-6, D-9,
  D-59): overlapped Windows enumeration + IOCP + `CreateThreadpoolIo` (`TP_IO`) +
  `TP_WORK`, and the Linux io_uring path where available, driven by the M7 unified
  park/resume continuation (D-59); as part of this, thread a **parent directory
  handle** through the enumeration backend so children open relative to it (openat /
  handle-relative `NtCreateFile`, D-9), replacing the current full-path open. Restore
  the `\?\` NT-layer open (dropped when M5-4 switched to `std::fs::OpenOptions`) so
  Windows **long paths (>260)** and **trailing-dot-space names** work — and add the
  M8-2 integration coverage for them (those files cannot be created through the Win32
  layer, so the tests belong here). **Gated on** building the D-5 completion
  abstraction + the native async FFI, not a missing consumer.

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
  pattern segment, push a name filter into the enumeration syscall
  (`FindFirstFileW` wildcard / `readdir` prefix) instead of enumerating the whole
  directory and matching in-process. Sound because the pattern is gospel (D-19); the
  in-process descend pruning already bounds *which directories* are scanned, so this
  is a pure per-directory perf add. **Gated on** profiling, not a missing consumer.
