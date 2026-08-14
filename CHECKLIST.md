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

- [ ] **M4-1. Leaf vocabulary** (D-56, D-66): `name` (exact / seg-glob / substring
  / extension / `∈ set`), `entry-type`, `reparse` (bool+tag), `attr-bitmask`,
  `size`, `timestamps`, `depth` — each a **signed** leaf.

- [ ] **M4-2. Evaluation**: flat `Vec<Leaf>` conjunction (D-67); emit per-pattern,
  descend per-query, composed with the glob set (D-66).

- [ ] **M4-3. Lazy metadata fetch-mask** (D-62): compute the union of referenced
  stat-tier fields across all emit lists ∪ descend ∪ `result_shape`.

- [ ] **M4-4. Tests**: ≥10 normal + edge cases over mock metadata; the
  `**/*.log`>10MB + `**/*.conf` per-pattern-emit example (D-66).

## M5 — Platform `sys` layer (safe wrappers over unsafe)

- [ ] **M5-1. Completion-based enumeration abstraction** (D-5, D-6): the trait a
  backend implements ("deliver next batch for this scan-state via a completion")
  + the scan-state object.

- [ ] **M5-2. Windows backend** (D-4, D-9, D-13): relative `NtCreateFile`,
  overlapped `NtQueryDirectoryFile` (inline attrs + reparse tag via
  `FILE_ID_EXTD_DIR_INFORMATION`), `CreateThreadpoolIo`/`TP_IO`, IOCP,
  `FILE_ID_INFO` for cycle keys (D-51).

- [ ] **M5-3. Linux backend** (D-6, D-9): `openat`, blocking `getdents64` on a pool
  thread, `statx`; io_uring for `openat`/`statx` where available; `(st_dev,
  st_ino)` cycle keys (D-51).

- [ ] **M5-4. Threadpool-work + waitable primitives**: `TP_WORK` wrapper (D-4) and
  the waitable-handle abstraction (Windows event / Linux eventfd, coalesced) for
  D-60.

- [ ] **M5-5. Integration test**: enumerate a large generated tree on the host OS;
  verify inline metadata, reparse detection, and correct handling of long / non-
  UTF-8 names (D-30, D-46).

## M6 — Ring & API surface

- [ ] **M6-1. CQ ring** (D-68, D-63): `crossbeam_queue::ArrayQueue` of owned
  `CqItem` (native inline name blob ≤512B); over-cap spill mechanism (D-63,
  resolve here).

- [ ] **M6-2. Backpressure + signaling wrapper** (D-11, D-60): full →
  park-continuation, drain → wake via `TP_WORK`; empty→non-empty coalesced signal;
  waitable-handle + non-blocking `drain()` + servicing adapters.

- [ ] **M6-3. SQ + CQ item types** (D-66, D-64, D-58, D-61): `SubmitQuery` /
  `Cancel` / `DecisionAnswer`; CQ enum `Match` / `ContainerEnter` / `ContainerEnd`
  / `Error` / `DecisionRequest` / `Terminal`; two monotonic id spaces.

- [ ] **M6-4. Query builder → core query-def** (D-31–D-34, D-66): builder accepts
  absolute patterns / CWD base and lowers to roots + relative patterns; fallible
  `submit` compiles the pattern set.

- [ ] **M6-5. Integration test**: ring ordering invariants (enter-before-children,
  nested ends — D-64), bounded no-drop backpressure, cancellation terminal marker
  FIFO ordering (D-61).

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
