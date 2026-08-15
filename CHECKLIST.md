# globazog — Build Checklist

Plan to implement the design in [DESIGN-NOTES.md](DESIGN-NOTES.md) (decisions
`D-1`…`D-68`). Built **bottom-up**: portable syntax/predicate layers first (no
I/O, fast to test), then the platform `sys` layer, the ring/API, the engine, and
finally end-to-end integration. Milestones end with integration tests where
natural. Cite the relevant `D-n` on each item.

End-of-milestone steps (repo standard, **not** listed as items): build the
default workspace debug+release with zero warnings; test the in-scope crate;
sync with origin and push.

Open design points to resolve *in situ* as their milestone is reached (not
blockers now): over-cap name spill mechanism (D-63), cancellation-accounting
counters (O-D′ / D-50), exact `sys` wrapper surface (O-E / D-54).

---

## M1 — Scaffolding & crate skeleton

- [x] **M1-1. Resolve crate instantiation.** Core lib crate = `globazog`; MCP crate
  + VS Code extension removed (deleted by owner); release-please kept (crates.io),
  Marketplace dropped. Workspace de-templated: placeholders resolved, single member,
  `cargo-generate.toml` / `scripts/post-script.rhai` / `README.template.md` /
  extension workflows removed, minimal buildable `globazog` crate created.

- [x] **M1-2. Module skeleton** in the core crate: `sys`, `syntax` (dialects + IR +
  matcher), `predicate`, `ring`, `engine`, `builder`, `error` — each a stub that
  compiles. Copyright header on every source file (repo rule).

- [x] **M1-3. Workspace dependencies**: `crossbeam-queue` (D-68), `bitflags`
  (attr/field masks), `thiserror` (error taxonomy, M1-4); `windows-sys`
  (`cfg(windows)`) and `rustix` (`cfg(target_os = "linux")`) for the enumeration
  backends (D-54). Wired ahead of use (M2–M7); test-fixture dev-deps added when
  fixtures land.

- [x] **M1-4. Error taxonomy** (`error` module): `Error` (fatal: `Pattern`, `Io`;
  `#[non_exhaustive]`) + `EntryError` (per-entry stream item, D-53), via `thiserror`.
  Container linkage attached when ring item types land (D-64).

- [x] **M1-5. Skeleton green**: crate builds debug+release with zero warnings;
  `tests/smoke.rs` passes (crate links + `Error` displays), picked up by CI
  `cargo test`.

- [x] **M1-6. Add crates.io publish workflow.** `.github/workflows/publish-crate.yml`
  triggers on the release-please `v*` tag and runs `cargo publish -p globazog --locked`;
  requires a `CARGO_REGISTRY_TOKEN` repository secret (**owner to set** before the
  first release). Marketplace publishing intentionally omitted.

## M2 — Glob matcher core (portable, no I/O)

- [x] **M2-1. char32 reversible decode** (D-46): `syntax::decode` — WTF-8 surrogate
  preservation (`decode_utf16`), PEP-383 surrogateescape (`decode_bytes`), lossless
  `decode_str`; never panics. 13 tests incl. unpaired surrogates, invalid/truncated
  UTF-8, empty.

- [x] **M2-2. Segment-structured IR** (D-18): `Token` (Literal / `?` / `*` / n-ary
  `Alt`), `Segment`, `PatternSegment` (`Match` / `DoubleStar`), `Pattern`
  (D-24, D-44, D-67).

- [x] **M2-3. Single- + cross-segment matcher** (D-46): `match_segment`
  (backtracking; `*` / `?` / alternation) and `match_path` (`**` zero-or-more,
  whole-segment) with the `CaseSensitivity` option (D-23). Case fold uses the
  Windows ordinal uppercase table (D-28, M2-6). 17 tests.

- [x] **M2-4. Path-structure primitives** (D-25, D-26): `syntax::path` —
  `split_segments` (collapses consecutive separators; UNC hook left to `win`, M3)
  and `is_dot` / `is_dotdot`. Application (`.`-strip / `..`-reject in pattern
  parsing, `.`/`..` enumeration skip) is wired in M3 / M7. 7 tests.

- [x] **M2-5. Literal-prefix / anchor extraction** (D-37): `syntax::anchor` —
  `literal_of` (segment → literal, for mid-pattern pruning D-34) and
  `literal_prefix` (leading literal seek target). 5 tests.

- [x] **M2-6. Windows ordinal uppercase-table case folding** (D-28). `syntax::upcase`
  — a 973-entry BMP delta table snapshotted from `RtlUpcaseUnicodeChar` via
  `tools/gen-upcase-table.ps1` (frozen/gospel) — replaces the interim ASCII fold;
  matches `CompareStringOrdinal(bIgnoreCase)`, no normalization. Latin-1 / Greek /
  Cyrillic fold, base letters not conflated (`a`≠`á`), supplementary / escaped
  identity. 4 fold tests.

## M3 — Dialects & pattern-set

- [x] **M3-1. Dialect registry** (D-15–D-20): `syntax::dialect` — `Dialect`
  (`Posix`/`Win`), `id()`, `Alphabet` flag, `default_case()`, `is_supported()`
  platform gate (D-47), and `resolve()` with `@`-partial-version binding. 4 tests.

- [x] **M3-2. `posix` front-end** (D-21): `syntax::parse` — `/`-only separators,
  `\` escape, UTF-8, `.`-strip / `..`-reject (D-26), `**` whole-segment (D-24),
  brace alternation (D-44), anchor detection. Lowers to the IR.

- [x] **M3-3. `win` front-end** (D-22, D-45): `/`+`\` separators, `{{`/`}}`
  brace-doubling escape, `Drive` / `Unc` (leading `\`) / `Root` anchors (D-25).
  The Windows-only policy gate (D-47) is `Dialect::is_supported()`, applied by the
  engine/builder so parse/match stay portable and testable. ~20 parser tests.

- [x] **M3-4. Pattern-set** (D-36–D-41): `syntax::set::PatternSet` — `add`
  (per-pattern case/dialect), `matches` → matched-pattern indices (D-40; bitset
  materialized at the ring, M6), and `should_descend` = the union descend decision
  (D-39) via a sound mid-pattern viability walk (D-34). Mixed dialects (D-41).
  Anchor merge/fork for traversal seeding is engine work (M6/M7). 4 tests.

- [x] **M3-5. Integration test** (`tests/dialects.rs`): parse + match `posix` and
  `win` over a 1,200-path corpus — verifies `**`, braces, case-insensitive `win`,
  anchors, and descend pruning. (Non-UTF-8 name matching is covered by the M2
  decode tests; exercised end-to-end in M8.)

## M4 — Predicates

- [x] **M4-1. Leaf vocabulary** (D-56, D-66): `predicate::Leaf` — `Name`
  (exact / glob / contains / extension via `name_*` ctors) + `NameInSet`,
  `IsType`, `IsReparse` / `ReparseTag`, `AttrsAllSet` / `AttrsAllClear`, `Size`,
  `Time`, `Depth`; signed via `negate` / `Cmp`. `EntryMeta` is the metadata view.

- [x] **M4-2. Evaluation**: `eval_leaf` / `eval_all` — flat conjunction (D-67),
  empty = vacuously true. Name leaves reuse `match_segment` (M2) for case rules.
  Per-pattern emit / per-query descend placement + composition with the glob set
  land in the query-def (M6) and engine (M7); this milestone is the evaluator.

- [x] **M4-3. Lazy metadata fetch-mask** (D-62): `MetaMask` bitflags +
  `required_fields` union across a conjunction; name/depth need no fetch. The
  engine unions emit ∪ descend ∪ `result_shape` (M6/M7).

- [x] **M4-4. Tests** (`predicate/tests.rs`): 11 tests over mock `EntryMeta`
  covering every leaf, negation, conjunction, the fetch-mask, and the D-66
  `**/*.log`>10 MiB vs `**/*.conf` per-pattern-emit example.

## M5 — Platform `sys` layer

**Re-planned during execution.** M5 starts with a **portable, safe `std::fs`
backend** that produces entries + metadata and unblocks the ring/engine (M6/M7),
plus the waitable primitive. The **native OS backends** and the **async
IOCP/threadpool orchestration** are sequenced as explicit follow-ups: the async
park/resume is the M7 scheduler's unified continuation mechanism (D-59), and the
native Linux backend is blocked on a Linux test environment (this host is Windows).

- [x] **M5-1. Enumeration primitive + metadata** (D-13, D-54): `sys::DirEntry` /
  `FileId` / `DirEntry::meta()` → `EntryMeta`, and `sys::enumerate_dir` — portable
  `std::fs`, symlink-aware, extracting type / reparse / size / times / attrs and
  file-id where the platform exposes them cheaply (Unix `dev`/`ino`).

- [x] **M5-2. Waitable primitive** (D-60): `sys::signal::Signal` — a coalesced
  binary signal for ring-readiness / backpressure wakeups. (Raw OS-handle exposure
  — Windows event / Linux eventfd — for foreign reactors is a native follow-up.)

- [x] **M5-3. In-crate enumeration + signal tests**: a ~210-entry temp tree
  (`tempfile`) enumerated with metadata assertions, plus signal coalescing /
  cross-thread wakeup. 5 tests.

- [x] **M5-4. Native Windows backend** (D-4, D-9, D-13, D-30): relative
  `NtCreateFile` + `NtQueryDirectoryFile` (`FILE_ID_EXTD_DIR_INFORMATION`: inline
  attrs, reparse tag, 128-bit file id), NT-layer long/`\?\` paths. Testable on this
  Windows host; slots in behind the M5-1 contract.

- [x] **M5-5. Native Linux backend** (D-6, D-9): `openat` + `getdents64` + `statx`
  via the safe `rustix::fs` wrappers, `makedev(dev)`/`ino` file ids, reversible
  byte-name decode (D-46), symlink-aware (no follow). Unblocked by a WSL2 Ubuntu
  toolchain; validated there (native-vs-portable parity + symlink tests).

## M6 — Ring & API surface

- [x] **M6-1. CQ ring** (D-68, D-63): `crossbeam_queue::ArrayQueue` of owned
  `CqItem`; owned code-point name blob (D-69). The fixed 512-byte inline blob and
  over-cap spill are deferred to M∞-1 (gated on the inline-buffer optimization,
  D-69) — the current owned-`Vec` name has no fixed cap, so spill is moot until
  then.

- [x] **M6-2. Backpressure + signaling wrapper** (D-11, D-60): full →
  park-continuation (`try_push` hands the item back), empty→non-empty coalesced
  signal, non-blocking `drain()` + `wait_pop`/`wait_nonempty` servicing surface.
  `TP_WORK`-driven wake and the raw waitable OS handle are native follow-ups with
  M7 (D-69).

- [x] **M6-3. SQ + CQ item types** (D-66, D-64, D-58, D-61): `SubmitQuery` /
  `Cancel` / `DecisionAnswer`; CQ enum `Match` / `ContainerEnter` / `ContainerEnd`
  / `Error` / `DecisionRequest` / `Terminal`; two monotonic id spaces (`IdSpace`).

- [x] **M6-4. Query builder → core query-def** (D-31–D-34, D-66): builder accepts
  absolute patterns / CWD base and lowers to roots + relative patterns; fallible
  `submit` compiles the pattern set.

- [x] **M6-5. Integration test**: ring ordering invariants (enter-before-children,
  nested ends — D-64), bounded no-drop backpressure, cancellation terminal marker
  FIFO ordering (D-61).

- [ ] **M∞-1. Zero-copy inline name blob + over-cap spill** (D-63, D-68, D-69):
  replace the owned code-point `Vec` name with a fixed inline native-unit buffer in
  a 512-byte descriptor slot, add the borrowed / lending-cursor read path, and
  implement the pathological over-cap spill (spill-to-side or error item).
  **Gated on** profiling showing the owned-copy cost matters (D-68/D-69), not on a
  missing consumer.

## M7 — Engine (the scheduler)

- [ ] **M7-1. Model B scheduler core** (D-3, D-7, D-8, D-12): permit-bounded scans,
  work queue, launch pump; unit of work = one directory.

- [ ] **M7-2. Relative-open + parent-handle refcount** (D-9, D-10): queue paths not
  handles; refcount cascade; `ContainerEnd` = refcount-zero (D-64); depth-first +
  enumerate-whole-dir-before-recurse (D-48).

- [ ] **M7-3. Unifying suspension** (D-59, D-11, D-58): one park/resume mechanism
  for I/O wait, backpressure, and `defer-to-client` escalation.

- [ ] **M7-4. Cancellation + accounting** (D-50, D-61): unified outstanding-work
  counters (resolve O-D′ here), `CancelIoEx`, single drain/teardown path, terminal
  marker; drop-handle RAII.

- [ ] **M7-5. Cycle detection + pushdown gating** (D-51, D-19): (volume-GUID/dev,
  file-id/ino) hash set when following reparse; conservative sound FS-filter
  pushdown at terminal segments.

- [ ] **M7-6. Native async enumeration backends** (D-4, D-6, D-59): overlapped
  Windows enumeration + IOCP + `CreateThreadpoolIo` (`TP_IO`) + `TP_WORK`, and the
  Linux io_uring path where available, driven by the M7 unified park/resume
  continuation (D-59). Moved here from M5-6: the async orchestration is the
  scheduler's mechanism, so it belongs with the engine, not the sync backends.

## M8 — End-to-end integration

- [ ] **M8-1. Wire it together**: syntax + predicate + sys + ring + engine behind
  the public builder/submit API.

- [ ] **M8-2. Large-scale integration tests**: thousands of files, deep trees,
  reparse points/junctions, long paths, non-UTF-8 / trailing-dot-space names
  (D-30, D-46); verify match correctness, ordering markers, backpressure, cancel.

- [ ] **M8-3. Example consumer**: a minimal end-to-end example (the tpu-mcp globber
  replacement shape, D-57) exercising multi-pattern + per-pattern emit + defer-to-
  client.

- [ ] **M8-4. Docs pass**: crate-level docs, the `win` brace-escaping section
  (D-45), and a DESIGN-NOTES cross-link.
