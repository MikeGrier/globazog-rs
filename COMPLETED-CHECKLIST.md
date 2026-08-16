# globazog — Completed Checklist

Append-only archive of completed [CHECKLIST.md](CHECKLIST.md) items. See
[CHECKLIST.md](CHECKLIST.md) for the remaining pending work.

## Moved 2026-08-15 — M1–M9 (matcher → dialects → predicates → sys → ring → engine → integration → per-entry/fatal errors)

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

## M7 — Engine (the scheduler)

- [x] **M7-1. Model B scheduler core** (D-3, D-7, D-8, D-12): permit-bounded worker
  pool (one permit = one worker = one live scan), LIFO work stack, launch pump; unit
  of work = one directory, matched/emitted inline (D-70).

- [x] **M7-2. Parent-handle refcount + queue-paths** (D-9, D-10): refcount cascade;
  `ContainerEnd` = refcount-zero, bottom-up (D-64); depth-first +
  enumerate-whole-dir-before-recurse (D-48); child paths queued, not handles (D-10).
  True parent-fd **relative-open** (openat) is deferred to M7-6 (needs the backend
  to accept a parent handle) — see M7-6.

- [x] **M7-3. Unifying suspension** (D-59, D-11): I/O wait and output backpressure
  are one mechanism — worker-thread blocking (the thread is the continuation, D-70),
  backpressure via a cancel-responsive timed wait (D-11). `defer-to-client` (D-58)
  is spawned as M7-7 (needs a tri-state predicate leaf).

- [x] **M7-4. Cancellation + accounting** (D-50, D-61): `outstanding` job counter
  (0 ⇒ complete) + the D-9 refcount resolve O-D′; single drain/teardown path;
  coordinator emits the terminal marker last (FIFO); `EngineHandle::drop` RAII
  cancel-drain-join. (`CancelIoEx` is a native-async detail → M7-6.)

- [x] **M7-5. Cycle detection + descend pruning** (D-51, D-19): `(volume, file-id)`
  visited set gating descent into reparse-point dirs; sound mid-pattern descend
  viability pruning (D-39). Syscall-level FS-filter pushdown at terminal literal
  segments (a perf add) is spawned as M∞-2.

## M8 — End-to-end integration

- [x] **M8-1. Wire it together**: syntax + predicate + sys + ring + engine behind
  the public builder/submit API, with a curated crate-root re-export surface
  (`globazog::{QueryBuilder, CqItem, Dialect, …}`).

- [x] **M8-2. Large-scale integration tests** (D-30, D-46): thousands of files (5000),
  deep trees (60 levels), mixed roots, bounded no-drop backpressure (2-slot ring),
  cancellation, and (unix) symlink-loop safety + non-UTF-8 names — match correctness,
  Enter/End nesting, and terminal markers verified. Windows **long-path (>260) and
  trailing-dot-space** coverage needs the native `\?\` open path (the files cannot
  even be *created* through the Win32 layer) → folded into M7-6.

- [x] **M8-3. Example consumer** (`examples/glob.rs`, D-57): the tpu-mcp globber shape
  — multi-pattern + per-pattern emit filter, printing matches and the terminal
  outcome. (`defer-to-client` awaits M7-7.)

- [x] **M8-4. Docs pass**: crate-level docs with a runnable-shape example, the `win`
  brace-doubling escape section (D-45), and a DESIGN-NOTES / `D-70` cross-link.


## M9 — Per-entry error propagation & fatal-error terminal (review-driven, PR #1)

Two distinct error semantics surfaced by PR review. **A** = a single bad entry must
not drop its readable siblings (a bug against the *existing* D-53 per-entry contract).
**B** = some errors must be able to *stop* the whole enumeration with a terminal
"stopped due to error" notification (a new capability; `TerminalReason` has only
`Completed`/`Cancelled` today). Sequenced A → B.

- [x] **M9-1. Per-entry failures don't abort a directory** (D-53): introduce
  `sys::DirScan { entries: Vec<DirEntry>, entry_errors: Vec<io::Error> }` returned by
  `enumerate` / `enumerate_dir` and the native backends. Per-entry `file_type` /
  `metadata` / `statx` failures are collected into `entry_errors` instead of aborting
  the directory via `?`; the outer `io::Result::Err` stays reserved for a
  directory-open/read failure. The Windows inline-metadata backend has no per-entry
  stat, so it returns an empty `entry_errors`. Update the `sys` unit tests.

- [x] **M9-2. Engine surfaces per-entry errors and continues** (D-53): the engine
  emits one `CqItem::Error` per `entry_errors` element (attributed to the container)
  and then processes the surviving entries. Tested at the portable seam
  (`read_one_entry` turns a failing entry into a collected error, not a `?` abort). A
  full end-to-end per-entry-error test is **not deterministically reproducible** — the
  Windows native backend has inline metadata (never fails per-entry) and a Linux
  `statx` failure is an inherent list/stat race — so it is covered at the seam rather
  than with a racy integration test (re-plan recorded during execution).

- [x] **M9-3. Fatal-error terminal** (new **D-71**; refines D-53, extends D-61): add
  `TerminalReason::Failed` as a **unit** variant (the error rides in a preceding
  `CqItem::Error`, so the reason stays `Copy`/`Eq`). The engine gains a `fatal` flag
  mirroring `cancel`: a **root-level (depth 0) enumeration-open failure** emits the
  error item, stops the walk, and the coordinator emits `Terminal{Failed}`; failures
  below the root remain per-container/per-entry and continue (D-53). Record **D-71**
  in [DESIGN-NOTES.md](DESIGN-NOTES.md) (with the adjacent refine markers on D-53 and D-61) in the same
  commit, including the recorded note that the initial fatal policy is depth-0 only
  (multiple-roots behavior is called out for future refinement). Integration test:
  an unopenable root ⇒ a `CqItem::Error` immediately followed by `Terminal::Failed`.

- [x] **M9-4. Contract docs pass**: update the crate-level error/terminal narrative
  ([lib.rs](crates/globazog/src/lib.rs)), the `CqItem::Error` / `TerminalReason`
  doc comments, and the [DEVELOPMENT.md](DEVELOPMENT.md) status to describe per-entry-continue vs.
  fatal-terminate. Ends the milestone (implicit build/test/sync gate follows).


## Moved 2026-08-15 — M10 (client-controlled symlink follow policy, D-72)

- [x] **M10-1. `FollowLinks` follow policy** (new **D-72**; resolves the D-13/D-51
  relationship): add a `FollowLinks { Never, Always }` enum and
  `Options.follow_links` (default `Never`), re-exported from the crate root. The
  engine descend gate treats a reparse entry as a directory *candidate* only under
  `Always` (both platforms), after which `should_descend` + the `descend` conjunction
  (D-56/D-66) filter it and `admit_descend` (D-51) cuts loops; a followed non-directory
  target surfaces as a per-entry `CqItem::Error` (D-53). Recorded **D-72** in
  [DESIGN-NOTES.md](DESIGN-NOTES.md) (default = D-13 never-auto-follow; D-51 is the loop guard when
  following is on; Windows no longer auto-follows dir-symlinks by default). Unit test
  (default `Never`) + unix integration test (a symlinked dir is descended only under
  `Always`).

## Moved 2026-08-15 — M11 (root lexical canonicalization + overlapping-root rejection, D-73)

- [x] **M11-1. Decide overlapping-root semantics** (D-73): resolved to *reject*
  overlapping **supplied** roots rather than silently drop or merge — dropping the
  descendant changes the match set (a non-recursive `*.c` matches under the deeper
  root but not the shallower), and the full enumerate-once/emit-under-both merge needs
  a multi-frame model (deferred to M∞-3). Recorded as **D-73** in
  [DESIGN-NOTES.md](DESIGN-NOTES.md); D-35/D-37 status markers updated.

- [x] **M11-2. Owned lexical root canonicalization**: `builder::canon_key` folds `.`,
  normalizes separators via `Path::components`, case-folds on Windows (D-28), and
  rejects `..` (D-26/D-35). `intern_root` dedups supplied and derived roots on that
  key (so `/data` ≡ `/data/.`, and `C:/Data` ≡ `C:/data` on Windows). Builder unit
  tests: `lexically_equal_roots_are_deduped`, `parent_dir_in_root_is_rejected`,
  `windows_roots_dedup_case_insensitively`.

- [x] **M11-3. Reject ancestor/descendant supplied-root overlap** (superseding the
  original "merge" plan per D-73): `build()` returns `Error::Options` when one supplied
  root is a lexical ancestor of another; derived (anchored-pattern) roots are exempt.
  Tests: `nested_supplied_roots_are_rejected`, `sibling_supplied_roots_are_allowed`.
  The full multi-frame merge is deferred to M∞-3.

## Moved 2026-08-16 — M12 (root confinement against reparse-point escape, D-75)

- [x] **M12-1. `Options.confine_to_roots` + builder** (D-75): added the bool option
  (default `false`), the `QueryBuilder::confine_to_roots` setter, and threaded it into
  `Query`. Unit test: `confine_to_roots_defaults_off_and_is_settable`.

- [x] **M12-2. `CqItem::Blocked` + `BlockReason`** (D-75): added the CQ variant, the
  `Blocked { container, name, reason }` struct, and the `#[non_exhaustive]`
  `BlockReason::RootEscape` enum; re-exported from the crate root; updated the CqItem
  doc tables (lib.rs / README).

- [x] **M12-3. Engine confinement check** (D-75): roots canonicalized once at spawn
  (when confine on); at a reparse-point descend candidate, the target is canonicalized
  and the descent declined (emit `Blocked{RootEscape}`, skip `launch_child`) when it is
  not within a canonical root or cannot be resolved (fail-closed). Component-wise
  ancestor test with the D-28 fold, reusing `sys::decode_name` + `matcher::fold`.

- [x] **M12-4. Tests** (D-75): unix integration `confine_to_roots_blocks_symlink_escape`
  — an escaping symlink is not descended and yields exactly one `Blocked{RootEscape}`
  naming it, an in-root symlink is still followed, and with `confine_to_roots(false)` the
  escaping link is followed and no `Blocked` is emitted.
