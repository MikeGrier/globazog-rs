# globazog — Build Checklist

Built **bottom-up** per [DESIGN-NOTES.md](DESIGN-NOTES.md). Completed milestones
**M1–M10** (matcher → dialects → predicates → `sys` incl. native Win/Linux backends →
ring → the synchronous Model B engine → end-to-end integration, example, docs →
per-entry error propagation + fatal-error terminal → client-controlled symlink follow
policy) are archived in
[COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md). Only the remaining pending work is
listed below.

End-of-milestone steps (repo standard, **not** listed as items): build the default
workspace debug+release with zero warnings; test the in-scope crate; sync with
origin and push.

## M11 — Root canonicalization & overlapping-root merge (D-35/D-37)

The builder's `intern_root` currently deduplicates only **byte-for-byte-equal** root
`PathBuf`s, so it does not yet realize D-35's owned lexical canonicalization nor
D-37's "no directory enumerated twice" collapse. Two roots where one is an ancestor
of the other (e.g. `.root("/tmp").root("/tmp/sub")`) enumerate the overlap twice and
can double-emit matches; lexically-equivalent roots (`/tmp/x` vs `/tmp/./x`) also stay
separate.

- [ ] **M11-1. Decide overlapping-root semantics** (new decision refining D-35/D-37):
  settle the owned lexical-canonicalization rules for roots (separator-fold; `.`-fold;
  the `..` policy given D-26/D-35 reject-not-resolve; case-fold on Windows per D-28)
  **and** how ancestor/descendant overlap composes with the D-38 relative-pattern
  cross-product — specifically whether an overlapped directory is physically enumerated
  once but matched under **multiple** `(root, root-relative)` frames, or the descendant
  root is dropped (which changes the emitted paths / root indices). Record the decision
  in [DESIGN-NOTES.md](DESIGN-NOTES.md) before coding. **Blocked on** a design decision
  (it changes public output semantics) — raise with the user, do not pick silently.

- [ ] **M11-2. Owned lexical root canonicalization**: implement the D-35 canonical form
  in the utility layer and dedup roots on it per M11-1 (so `/tmp/x` ≡ `/tmp/./x`, and
  separator/case-equivalents merge). Builder unit tests for each rule.

- [ ] **M11-3. Ancestor/descendant traversal merge**: per M11-1, enumerate each unique
  physical directory once while preserving every applicable `(root, rel)` frame, so an
  overlapped subtree is neither scanned nor emitted twice (D-37). Integration test:
  `.root(a).root(a/sub)` with a relative pattern enumerates the overlap once with the
  agreed match set. Ends the milestone (implicit build/test/sync gate follows).

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
  pattern segment, push a name filter into the enumeration syscall
  (`FindFirstFileW` wildcard / `readdir` prefix) instead of enumerating the whole
  directory and matching in-process. Sound because the pattern is gospel (D-19); the
  in-process descend pruning already bounds *which directories* are scanned, so this
  is a pure per-directory perf add. **Gated on** profiling, not a missing consumer.
