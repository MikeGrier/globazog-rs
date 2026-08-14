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

- [ ] **M1-6. Add crates.io publish workflow.** A job (or `publish-crate.yml`)
  triggered on the release-please `release_created` / `v*` tag running
  `cargo publish -p globazog`; requires a `CARGO_REGISTRY_TOKEN` repository secret
  (owner to set). Marketplace publishing intentionally omitted.

## M2 — Glob matcher core (portable, no I/O)

- [ ] **M2-1. char32 reversible decode** (D-46): WTF-8 surrogate preservation
  (Windows `[u16]`) and PEP-383 surrogateescape (Linux `[u8]`) into a code-point
  stream; never panics on ill-formed input. ≥10 normal + edge cases (unpaired
  surrogates, invalid UTF-8, empty).

- [ ] **M2-2. Segment-structured IR** (D-18): per-segment matcher over char32 —
  literal / `*` / `?` / **n-ary** brace alternation (D-44, D-67); cross-segment
  `**` with zero-match and whole-segment-only rule (D-24).

- [ ] **M2-3. Single-segment match engine** + simple Unicode case-fold (D-28) and
  the case-sensitivity option (D-23). ≥10 normal + edge cases.

- [ ] **M2-4. Path-structure handling** (D-25, D-26): separator/segment split,
  consecutive-separator collapse (with UNC-anchor exception hook), `.` strip /
  `..` reject, `.`/`..` enumeration-entry skip.

- [ ] **M2-5. Literal-prefix / anchor extraction** (D-37) for a single pattern;
  mid-pattern literal viability pruning (D-34). Unit tests.

## M3 — Dialects & pattern-set

- [ ] **M3-1. Dialect registry** (D-15, D-16, D-17, D-20): enum + ASCII id +
  alphabet flag (ASCII rejects >127); `@`-versioning with partial-version binding
  and resolve-to-concrete.

- [ ] **M3-2. `posix` front-end** (D-21): `/`-only separators, `\` POSIX escape,
  UTF-8, default case-sensitive. Lowers to the IR.

- [ ] **M3-3. `win` front-end** (D-22, D-45, D-47): `/`+`\` separators, brace-
  doubling escape (`{{`/`}}`), UNC leading-`\\` anchor (D-25), Windows-only gate,
  default case-insensitive. Document brace escaping prominently.

- [ ] **M3-4. Pattern-set compilation** (D-36–D-39): anchor merge/fork,
  per-directory live-set, matched-pattern bitset (D-40); mixed dialects in one set
  (D-41).

- [ ] **M3-5. Integration test**: parse + match both dialects over a large
  generated name corpus; verify anchors, `**`, braces, case, UNC, non-UTF-8 names.

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
