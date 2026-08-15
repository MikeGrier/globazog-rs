# globazog — Design Notes

(Crate name **globazog**; repository is `globbinobulous-rs`.)

Status: **design in progress** (no code yet). This file records what has been
**decided** and what remains **open** from the design conversation. Decisions are
given stable IDs (`D-n`) so later checklists / code can cite them. "Pattern is
gospel" and "we define our behavior, dependencies merely satisfy it" (repo
Design-Autonomy rule) are load-bearing throughout.

Last updated: 2026-08-14.

---

## 1. Purpose & scope

- **D-1. What it is.** A high-performance, **Windows-first** globbing + directory-
  traversal library for Rust. The differentiator over existing crates
  (`glob`, `globset`, `ignore`, `jwalk`, `walkdir`, `wax`) is the *combination* of
  (a) **bounded parallel** directory scanning with a **metadata-aware
  filter-before-recurse**, and (b) an **owned, multi-dialect** glob-syntax layer
  whose semantics we fully control.
- **D-2. Core value proposition.** Bounded concurrency sized to overlap I/O
  latency, *not* to parallelize cheap CPU work; the descend decision is a
  first-class, metadata-aware predicate (not gitignore-shaped, not path-only).
- **D-57. Primary consumers (drive the API shape).** A replacement for the
  globber inside **tpu-mcp**; a planned **usermode filesystem project** built on
  ioring-based I/O; and general reuse across existing/future projects. This
  consumer set is ioring-based, which is *why* the public API is ring-shaped
  (D-55) — it is consumer-driven, not speculative. (Resolves O-18.)

---

## 2. Concurrency & traversal engine

- **D-3. Scheduler = Model B (async completion), not a blocking pool.** In-flight
  I/O concurrency is decoupled from thread count.
- **D-4. Windows backend = IOCP via the Win32 threadpool** (`CreateThreadpoolIo` /
  `StartThreadpoolIo`), driving **overlapped `NtQueryDirectoryFile`**.
  - Two threadpool primitives: **`TP_IO`** for enumeration completions (one per
    directory handle, `CloseThreadpoolIo` at scan end; strict
    `StartThreadpoolIo`-before-each-op / `CancelThreadpoolIo`-on-sync-failure
    pairing), and **`TP_WORK`** for the launch pump and for **resuming
    continuations parked on backpressure**.
- **D-5. Completion-based enumeration abstraction.** The OS enumeration backend is
  hidden behind a *completion* contract ("here is a scan-state; deliver its next
  batch of entries via a completion"). Scheduler, matcher, and prune layers are
  OS-agnostic and never see the backend.
- **D-6. Linux backend is pragmatic, not symmetric.** `getdents64` has **no
  io_uring opcode**, so core enumeration runs as a **blocking syscall on a pool
  thread** behind the same completion contract. io_uring is used only where it
  helps: async `openat` / `statx` / `close`. The abstraction does **not** pretend
  the two OSes are symmetric.
- **D-7. Unit of parallel work = "scan one directory," never "process one
  entry."** All cheap work — glob-matching a name, testing the reparse bit,
  emit/descend — happens **inline** on the worker holding the entry buffer. It
  never gets its own thread/task/queue hop.
- **D-8. Bounded in-flight scans via a permit/semaphore.** The "number of parallel
  scans" knob. **One permit = one live scan = one open directory handle** (a scan
  parked on backpressure still holds its permit — correct, it is still consuming
  the bounded resource).
- **D-9. Relative-open + parent-handle refcount.** Children opened relative to the
  parent directory handle (`openat`-style / dir-handle-relative `NtCreateFile`):
  faster, avoids long-path re-parsing, dodges TOCTOU, sidesteps `\\?\` / 260. The
  parent handle is refcounted by "outstanding unscanned children" and released
  (cascading up) when the last child completes. Open ancestor handles are bounded
  by **tree depth**, not breadth.
- **D-10. Queue paths, not open handles.** A discovered-but-unscanned directory is
  a name/path in the queue; the handle opens only under a permit. Bounds open
  handles to in-flight scans.
- **D-11. Backpressure = bounded results ring, suspend-the-continuation.** A full
  output **parks the scan continuation** (stops pulling entries, resumes on drain)
  and returns the thread to the port — it does **not** block a completion thread
  (which would starve the IOCP). The output path is therefore **custom** (wakes a
  parked producer via `TP_WORK`); a stock channel cannot do this. See D-55, D-59.
- **D-12. Queue topology (working model).** Roots in: MPSC. Internal work queue:
  **MPMC / work-stealing**. Results out: the response ring (D-55). The earlier
  "spmc/spsc for results" framing was mis-specified; results are many-producer.
- **D-13. Prune predicate is metadata-rich.** May use path *or* the entry's inline
  metadata; **is-reparse-point** is the primary interest. On Windows it is
  **free** (`FILE_ATTRIBUTE_REPARSE_POINT`, plus reparse tag via
  `FILE_ID_EXTD_DIR_INFORMATION`, returned inline). An extra `statx` on Linux for
  such metadata is acceptable.
- **D-62. Lazy metadata fetch.** Only pay a Linux `statx` (or a Windows extra
  query) when a submitted predicate leaf — or the emitted result shape — actually
  references a `statx`-tier field (size / timestamps / sometimes type or reparse).
  Never stat every entry unconditionally; the submission's referenced fields drive
  per-entry metadata fetch. (Windows conditions are almost all free-inline.)
  *Realized by `sys::EnumPlan { want_stat, want_file_id }`, computed once by the
  engine from the fetch mask (`want_stat` = the mask intersects the size/time/attr
  fields) and the follow/cycle settings (`want_file_id` = cycle detection on **and**
  `FollowLinks::Always`, needed only to loop-guard a followed reparse point). The
  Linux backend keeps `getdents64`'s `d_type` for type/reparse and calls `statx` only
  when the plan needs a field or `d_type` is `Unknown`; the portable backend skips its
  per-entry `lstat` likewise. The Windows listing is inline, so the plan is a no-op
  there.*
- **D-14. Two orthogonal predicates** (revised from three): *emit this entry?* and
  *descend into this dir?* — **prune is not a separate predicate**: pruning a
  subtree ≡ `descend = false` (as in `find -prune`), so a user prune-rule is just
  a negative descend leaf. Delivered declaratively (D-56), with an optional ring
  escalation (D-58).
- **D-48. Enumerate the whole directory before recursing.** Drain all batches on
  the hot directory handle first (locality), then descend **depth-first (LIFO)**
  among the collected child directories. (Resolves O-9.)
- **D-50. Cancellation + unified outstanding-work accounting is first-class.**
  Termination and cancellation are the same problem: queued dirs + in-flight
  async ops + parent refcounts must all quiesce; cancel = stop launching,
  `CancelIoEx` outstanding ops, then **drain the aborted completions through one
  teardown path** (releasing handles/permits, unwinding refcounts). Triggered by
  D-61; accounting specifics still TBD (O-D′). (Resolves O-11 in principle.)
- **D-51. Cycle/loop detection** via a hash set keyed on **(volume GUID, file
  id)** — Windows `FILE_ID_INFO` (volume serial/GUID + 128-bit file id); Linux
  `(st_dev, st_ino)`. Engaged when the client's policy follows reparse points.
  (Resolves O-12.)
- **D-52. No default reparse follow policy.** The client decides via submitted
  policy (which sees reparse status, D-13); the library never auto-follows.
  Realized by the `FollowLinks` option, **D-72**. (Resolves O-13.)
- **D-53. Errors are surfaced as items in the output stream**, never aborting the
  walk (e.g. a permission-denied subtree becomes an error item). (Resolves O-14.)
  A **fatal** subset (a root that cannot be enumerated at all) instead stops the walk
  with a `Terminal{Failed}` acknowledgement — see **D-71**.

Windows caveat on record (not a decision): even the relative `NtCreateFile`
**open** effectively completes **synchronously** — only the *enumerate* overlaps.
Each launch costs a brief synchronous open on a pool thread; io_uring's `OPENAT`
lets Linux async the open where Windows cannot.

---

## 3. Glob-syntax architecture

- **D-15. Multiple named syntaxes (dialects), each well-defined.** The set is
  **closed for now** (no extension mechanism). Evolution is by **refinement**
  (D-20), not third-party additions.
- **D-16. Dialect identity.** Enum variant (compile-time exhaustiveness) +
  canonical short **ASCII id** + **alphabet flag** (ASCII | UTF-8).
- **D-17. Alphabet rules.** An **ASCII** dialect validates and **rejects bytes >
  127** at parse, and **must** define a supra-127 escape scheme (author's choice;
  spec-conformance, not runtime-validated). A **UTF-8** dialect accepts full
  `&str` and *may* offer escaping. The flag governs **pattern text**, not the
  **match target**.
- **D-18. One shared compiled IR.** Every dialect is a **front-end** lowering a
  UTF-8 pattern into one **segment-structured** IR (per segment: literal-prefix
  hint, single-segment matcher, cross-segment `**` flag). The engine is
  **dialect-agnostic** below the parse line.
- **D-19. Pattern is gospel; pushdown only accelerates.** The user's pattern is
  the sole source of truth; filesystems merely enumerate; **we** match
  authoritatively. FS-level filtering is a **conservative, per-segment, sound**
  optimization: may return *extra* entries (we re-filter), must **never drop** a
  true match, **never changes results**; where soundness is unprovable, push
  nothing. **Auto-gated by the engine; no caller force/forbid knob in v1**
  (O-5 resolved). Sound essentially only at terminal (leaf) segments; highest
  value on **large + remote (SMB) + selective** directories.
- **D-20. Dialect versioning.** Ids versioned with **`@`** (O-4): `posix@1.0`.
  **Partial-version binding:** `foo` → highest overall, `foo@1` → highest `1.*`,
  `foo@1.0` → highest `1.0.*`. **Refinement is additive/versioned, never
  mutating.** Degree of pinning = the user's opt-in to drift-vs-stability
  (`foo` = latest/accepts refinement; `foo@1.0.2` = pinned/gospel). Anything
  **persisted for stability records the resolved concrete version**. (Introducing
  a new metacharacter — e.g. real `[...]`, D-44 — changes an existing literal's
  meaning and is therefore a version bump.)

---

## 4. The two initial dialects

Both lower to the same IR; they differ **precisely in separator policy** and the
escaping that follows.

- **D-21. Dialect `posix` ("`/`-only", Linux-style).** `/` is the only separator;
  `*` single-level, `**` recursive. Alphabet **UTF-8**. Default **case-sensitive**.
  `\` = **POSIX escape** (`\*` → literal), confirmed (O-2).
- **D-22. Dialect `win` ("`/`+`\`", Windows-style).** Both `/` and `\` are
  separators; `*`/`**` as above. No backslash escape (it is a separator); see
  D-45 for escaping. Alphabet **UTF-8**. Default **case-insensitive**.
- **D-23. Case-sensitivity is an orthogonal runtime option; the dialect supplies
  its default** (`posix` = CS, `win` = CI), overridable. No `win-ci`/`win-cs`
  twins.
- **D-47. `win` is Windows-only** — rejected/unsupported on other platforms
  (policy gate, not a technical limit). `posix` is the portable dialect.
  (Resolves O-8.)

---

## 5. Pattern semantics (dialect-independent, in the IR)

- **D-44. v1 metacharacter set = `*`, `**`, `?`, `{…}`.** Brace alternation is
  **intra-segment only** (no separators inside a brace group), so it lowers to
  **alternation inside the single-segment matcher** — cheap, no whole-pattern
  expansion, and `*.{c,h}*` keeps **one** caller-facing pattern identity/tag.
  A brace group is **always** an alternation, even a single-alternative `{x}`
  (which matches `x`) — braces are **not** comma-gated (deliberate divergence
  from bash), so literal braces always require escaping (D-45). Full `[...]`
  character classes (ranges, `[!…]` negation — the de-facto standard in
  `glob`/`globset`/`wax`, O-6) are **deferred to a future dialect version**
  (`posix@2` / `win@2`), because making `[` a metacharacter changes its current
  literal meaning (D-20). (Resolves O-1.)
- **D-45. Escaping (confirmed, O-A).** `posix`: `\` escapes anything. `win`: since
  `* ? / \ : " < > |` are **illegal in Windows filenames**, the only
  metacharacters that can occur literally in a name are `{`/`}`, escaped by
  **doubling** (`{{` → `{`, `}}` → `}`, exactly like Rust format strings); `[`
  and `]` are ordinary literals in v1. Because a single `{…}` is always an
  alternation (D-44), literal braces **always** require doubling:
  `report{final}.txt` matches `reportfinal.txt`, while `report{{final}}.txt`
  matches `report{final}.txt`. **The `win` dialect spec must document brace
  escaping prominently.** **Limitation:** brace-doubling is recognized only
  *outside* a brace group; inside an alternation arm (`{…}`), `{{`/`}}` are **not**
  honored — the first `}` closes the group and `{` is a rejected nested-group start —
  because honoring them there is ambiguous with arm termination (`{a,b}}` could not
  close). Literal braces inside an alternation arm are therefore unsupported in v1.
- **D-24. `**` semantics.** Matches **zero-or-more** segments (`a/**/b` matches
  `a/b`); legal **only as a whole segment** (embedded `x**y` is an error); `a/**`
  matches `a` itself **and** everything beneath; `**/x` matches `x` at any depth
  **including** directly under root; consecutive `**` collapse.
- **D-25. Empty / consecutive separators.** Runs collapse to one — **except** a
  *leading* doubled separator in `win` is a **UNC anchor** (`\\server\share`),
  preserved.
- **D-26. `.` and `..`.** In a pattern: `.` is **stripped**, `..` is **rejected**
  (we do **not** resolve `..`). Enumeration's own `.` / `..` entries are **always
  skipped**.
- **D-46. Matching occurs in 32-bit code-point (UCS-4/char) space.** Both pattern
  and name are decoded to code-point sequences; case-fold equivalence classes are
  computed in that space; state machines decode **without panicking**. Decoding
  must be **reversible / non-lossy** — **WTF-8-style surrogate preservation** for
  Windows UTF-16 (unpaired surrogates), **PEP-383 surrogateescape** for invalid
  UTF-8 bytes on Linux — so no real filename becomes unmatchable/unopenable. A
  valid-UTF-8 pattern *literal* never equals an escaped-invalid code point (fine);
  `*`/`?` still match them. (Resolves O-7 and the parked name-side encoding.)
- **D-27. No Unicode normalization in v1.** Composed vs decomposed do not match.
  **Conscious deferral**, revisit before 1.0.
- **D-28. Case-insensitive = Windows ordinal uppercase-table fold** — matching
  `CompareStringOrdinal(bIgnoreCase)` / .NET `OrdinalIgnoreCase`, which Microsoft
  recommends for **file paths**. Fold = **uppercase each code point via the OS
  uppercase table** (`RtlUpcaseUnicodeChar`), then compare ordinally; **no
  normalization** (D-27) — so `'á'`↔`'Á'` match, `'a'`≠`'á'`, and composed vs
  decomposed never match. Folding is **per BMP code point**; surrogate /
  supplementary / surrogate-escaped values fold to themselves (identity),
  consistent with the code-point matcher (D-46). The table is **snapshotted once
  and embedded** (frozen → gospel; it does not drift with OS/Unicode updates, and
  may differ from a specific NTFS volume's `$UpCase` — acceptable, we own
  semantics), and is used for all case-insensitive matching.
  *(Supersedes the earlier "Unicode simple case folding" phrasing; the interim
  ASCII fold in `syntax::matcher` is replaced in M2-6.)*
- **D-29. Leading dot is not special**; `.` is an ordinary literal. **Hidden
  filtering is a separate predicate** (deliberate divergence from bash).
- **D-30. Trailing dots/spaces are ordinary ordinal characters.** `foo ` matches
  a pattern literally carrying the trailing space, not `foo`. No platform
  normalization. Riding the NT layer lets us see/match/open such names (and long
  paths) that `GetFullPathName`-based globbers structurally cannot — a headline
  completeness win.

---

## 6. Anchoring, roots, and no ambient state

- **D-31. Engine core does *zero* path resolution.** It takes **roots** (physical
  seeds) × **relative patterns** and evaluates each relative pattern against the
  path *relative to its seed*. No CWD, no canonicalization, no absolute-pattern
  handling in the core.
- **D-32. No process-global state, ever.** Never reads the process CWD or per-drive
  `=C:` / `=D:` env vars (consistent with the relative-open engine). Chosen
  because such state is a hazard in a highly concurrent engine.
- **D-33. Anchoring model.** A pattern is **self-rooting** if it carries an
  absolute anchor (leading `/`; or drive / UNC in `win`), else **relative** to the
  caller's root(s). `C:foo` (drive-relative) is an **error** unless an explicit
  per-drive base is supplied — never the env var.
- **D-34. A utility layer above the engine** handles all resolution: CWD → root
  (the one honest one-shot global read, at the caller's edge) and absolute-pattern
  **leading-literal-prefix → root** peeling. **Leading-prefix → root is the
  utility's job; mid-pattern literal pruning stays engine-core** (e.g. `foo` in
  `**/foo/*.c`).
- **D-35. Canonicalization is owned, not delegated.** No `GetFullPathName` (CWD /
  per-drive globals; strips trailing dots/spaces) and no blind delegation to
  lexical canonicalizers like `PathCchCanonicalizeEx` (they **resolve `..`**,
  contradicting D-26). Root canonicalization (for **anchor dedup/merge**) lives in
  the utility layer using our *owned* lexical logic (`.`-fold, `..`-reject,
  separator-fold); an OS API may at most be a cross-check.

---

## 7. Roots × patterns orthogonality (the pattern-set model)

- **D-36. The unit of matching is a *set* of patterns over a *set* of roots.**
  Single-pattern is the degenerate N=1 case. `*.c*` + `*.h*` enumerates **once**.
- **D-37. One deduplicated traversal.** Each pattern contributes a literal-prefix
  **anchor**; shared-prefix anchors **merge** into one seek, divergent anchors
  **fork**; overlapping roots/anchors collapse (no directory enumerated twice).
- **D-38. Relative patterns are root-independent templates applied at every root**
  (logical cross-product), still one physical enumeration per unique directory.
  Anchored patterns self-root and ignore supplied roots. *Realized by a per-pattern
  applicable-root set (`PatternEntry.roots`): a relative pattern lists every supplied
  root, an anchored pattern only its derived root; each scan carries its root index
  and `matches`/`should_descend` consider only patterns applicable to it.*
- **D-39. Descend is the union across the set.** Descend if *any* live pattern
  still wants the directory; stop (do not descend) only when dead for **every**
  pattern. The user descend-predicate (D-56) is ANDed on top.
- **D-40. Each result item is tagged with the matched-pattern set** (bitset over
  pattern indices). Brace alternation stays one pattern id (D-44).
- **D-41. Mixed dialects in one set are allowed** (falls out of the shared IR).
- **D-42. Exclusion / negative patterns are OUT for now** (door open).

---

## 8. Public API, platform layer, output protocol

- **D-54. Layered architecture with a platform `sys` layer.** Sharp `unsafe`
  per-platform syscall APIs (IOCP / `NtQueryDirectoryFile` / `CreateThreadpoolIo`;
  io_uring / `getdents64` / `openat` / `statx`) are exposed behind **safe
  wrappers**; the engine and everything above is safe Rust over those wrappers.
  (Addresses O-16.)
- **D-55. Public API = an io_uring-shaped submit/complete ring.** The client
  submits a query (a single structured block: roots + patterns + the two
  predicate exprs), and the library allocates a **response ring the client must
  service**. Matches, **end-of-container markers** (D-49), **error items** (D-53),
  and **decision-requests** (D-58) all flow through it. The public ring is **our
  own portable abstraction** (IOCP/threadpool-backed on Windows; closer to native
  on Linux), *not* the OS io_uring. (Resolves O-17.)
- **D-49. Output is unordered and carries end-of-container markers.**
  End-of-directory / end-of-subtree markers (alongside matches and error items)
  let a consumer impose ordering or detect completion boundaries. (Resolves O-10.)
- **D-63. Ring payload model = container-id + inline native-name blob (O-C′ #1).**
  A CQ item carries **(container-id, entry-name)**, never a full path. The
  **entry-name is an opaque native code-unit blob shipped verbatim from the FS** —
  `[u16]` on Windows (raw; *not* guaranteed well-formed UTF-16, unpaired
  surrogates possible), `[u8]` on Linux (*not* guaranteed UTF-8) — with no
  validation or conversion at emit. It is bounded by `NAME_MAX` (Windows ≤255
  units = **510 bytes**; Linux ≤**255 bytes**), so it **fits inline in a 512-byte
  descriptor slot — no buffer pool on the hot path**. Stored as
  `{ bytes, len, platform-tag }` (an `OsStr`-like opaque blob, **not** `str`; same
  shape in the C-ABI form). Full paths are reconstructed **client-side** by walking
  the container chain — mirroring the engine's parent-handle chain (D-9) and the
  client's ordering tree (D-49); a convenience adapter flattens to a path /
  converts to `String` via the reversible WTF-8 / surrogateescape of D-46. The raw
  bytes shipped are exactly the bytes the matcher decoded (D-46) — one
  representation, two readers. A pathological over-cap name (exotic/network FS)
  takes a rare **spill** path (spill-to-side or error item — mechanism TBD).
- **D-64. Container model (O-C′ #6).** A **container** is a scanned directory (or
  a root). It rides two CQ items, and both are just engine events surfaced on the
  ring:
  - **`ContainerEnter { id, parent, name }`** at **scan-start** (permit acquired,
    handle opened). `parent` = parent container-id or a `ROOT` sentinel; `name` =
    the inline native-blob (D-63), or a **root-index** into the submitted roots
    list when `parent = ROOT` (avoids inlining a long absolute root path).
  - **`ContainerEnd { id }`** at **refcount-zero** — the D-9 parent-handle
    refcount-zero / handle-release event surfaced on the ring. This is the
    **only** end semantics (subtree-complete); **1:1 with Enter**, emitted as a
    **bottom-up cascade** (never merged). A shallow "listing-complete" marker was
    **rejected**: a mid-walk snapshot is racy and implies a stability we cannot
    promise — false economy.
  - **`Match { container = parent, name-blob, matched-pattern bitset, requested
    metadata }`** is emitted at **discovery** (inline during the parent's
    enumeration, no permit), **not folded** into `ContainerEnter` — folding would
    couple match latency to descend scheduling and fails in the
    emit-without-descend case. A matching-and-descended dir yields a `Match` (at
    discovery) and a `ContainerEnter` (at scan-start), with different container-ids
    and meanings — not a duplicate.
  - **IDs:** two **monotonic-u64** spaces per query — **container-ids** (path tree)
    and **decision-tokens** (D-58 `defer-to-client`; kept distinct because a
    deferred entry may be vetoed and never become a container).
  - **Path reconstruction** is client-side: keep `container-id → (parent, name)`,
    walk to a `ROOT` (→ `root[i]`), prepend. The live map is bounded ≈
    **permits + depth** (a container is announced only when actually scanned under
    a permit) and shrinks from the leaves as `ContainerEnd`s arrive.
  - **Ordering invariants (hold on the multi-producer CQ):** (1) a `ContainerEnter`
    is enqueued before its id is ever used as a parent for a child launch, so FIFO
    ticketing delivers enter-before-children even across worker-producers; (2)
    `ContainerEnd`s cascade bottom-up, so a child's end precedes its parent's, the
    client drops on each end, and ancestors outlive descendants. Sibling containers
    interleave freely (D-49), demultiplexed by container-id.
  - **Roots** are announced at **actual enumeration start**, never at submission
    (bounded parallelism may defer a root's scan; the wire stays faithful).
- **D-65. ABI target = Rust-native (O-C′ #8).** The public ring surface is
  idiomatic Rust — enums-with-data, generics, lifetimes, `OsStr`-like name types —
  **not** a `#[repr(C)]` C-ABI ring (no cbindgen, no ABI-stability burden;
  versioned by Rust semver). CQ items are a Rust `enum` (Match / ContainerEnter /
  ContainerEnd / Error / DecisionRequest / Terminal); the D-63 name blob is a
  native Rust type (e.g. `enum { Windows(&[u16]), Unix(&[u8]) }`); ids are typed
  newtypes / `NonZero`. The usermode-FS consumer (D-57) is therefore Rust (or
  shims to it). The internal ring buffer still has a concrete layout but need not
  be `#[repr(C)]` (only Rust reads it). The D-60 waitable primitive is still
  exposed as a raw OS handle (`HANDLE` / `RawFd`) for foreign-reactor integration.
  Opens a zero-copy option: CQ items may be **borrowed views** into the ring (a
  lending cursor) rather than owned copies — a Rust-API ergonomics sub-choice (#3).
- **D-66. Query definition + compilation (O-C′ #5 / #4).**
  - **`descend` is per-query** (one global traversal-policy conjunction: reparse /
    cycle / prune-by-name), ANDed with the pattern-set union (D-39). **`emit` is
    per-pattern** (each entry's own output filter; empty = pass-all): match-bitset
    (D-40) bit *p* is set iff `p.glob matches AND all(p.emit)`.
  - Predicates are **flat `Vec<Leaf>` conjunctions** (AND-only, D-56; no tree —
    D-67).
  - **`result_shape`** = a `FieldMask` gating lazy-`statx` (D-62) over a fixed
    superset metadata slot. The engine's actual fetch-mask = union of every
    pattern's `emit` fields ∪ the query `descend` fields ∪ `result_shape`.
  - **Compilation folds into a fallible `submit`** (pattern errors surface there).
    **No separate compiled artifact in v1** — patterns are trivial (simple globs,
    *not* regexes; the only build cost is the combined-set structure, cheap at
    small scale, and useful cases should map to FS-level pushdown, D-19). The
    internal compiled-set representation is kept **separable**, so a reusable
    `PreparedQuery` / `CompiledSet` can be lifted out later for large-rule-set
    daemon reuse (validate-once / reuse, not CPU) — a non-breaking add.
  - **Builder vs core:** the public API is a **builder** that accepts absolute
    patterns / a CWD base and lowers (per D-34) to the **core** query-def, which
    holds **roots + relative patterns only**; the struct below is the builder's
    output.
  - **Options:** permit count (D-8), ring/buffer capacities, over-cap spill config
    (D-63), per-pattern case-sensitivity override (dialect default, D-23; with a
    query-level default), cycle-detection toggle (D-51). Ordering is fixed
    (unordered, D-49).
  - Sketch:

    ```rust
    struct PatternEntry { glob: CompiledPattern, dialect: DialectVersion, emit: Vec<Leaf> }
    struct Query {
        roots: Vec<Root>,
        patterns: Vec<PatternEntry>,
        descend: Vec<Leaf>,
        result_shape: FieldMask,
        options: Options,
    }
    ```
  - **SQ op set (#4):** `SubmitQuery` (fallible), `Cancel` (D-61), `DecisionAnswer`
    (D-58).
- **D-67. n-ary flattening principle.** Associative operators are represented
  **n-ary (flattened)**, never as nested binary pairs: predicate `AND` is a flat
  `Vec<Leaf>` (D-56 / D-66), brace alternation is an n-ary alternation node in the
  matcher (D-44), and any future `OR` would be n-ary likewise.
- **D-68. CQ ring = owned items on `crossbeam_queue::ArrayQueue` (O-C′ #7 +
  owned-vs-borrowed).** The completion queue is a **bounded lock-free MPMC ring**
  built on `crossbeam_queue::ArrayQueue` (Vyukov) — we *define* the contract
  (bounded, no-drop, many worker-producers + client consumer) and *choose*
  `ArrayQueue` because it satisfies it. Items are **owned**, popped **by value**
  (copy-out; a `CqItem` is ~512 B with the inline name, D-63 — cheap next to the
  enumeration syscalls). **Zero-copy borrowed / lending-cursor is deferred** (a
  later optimization needing a read-in-place head/tail index ring instead of
  `ArrayQueue`; only if profiling shows the copy matters). The **wrapper policy is
  ours, not off-the-shelf:** full → park-continuation + drain → wake via `TP_WORK`
  (D-11), and empty → non-empty waitable-handle signaling, coalesced (D-60). The
  **SQ** is low-volume/asymmetric and needs no lock-free ring — a mutex-guarded
  queue / direct submit for its three ops (D-66) suffices.
- **D-56. Predicates are declarative data submitted with the query, not client
  callbacks (confirmed, O-B / O-B′) — the inline fast path.** Two expressions
  (emit, descend — D-14), each an **AND-only conjunction of *signed* leaves**. No
  OR / NOT / XOR combinators are needed because (a) every leaf carries its own
  polarity/comparison (so NOT is unnecessary), (b) leaves may be **set-membership**
  (`name ∈ {…}`, absorbing bounded OR), and (c) name/path OR is the pattern set's
  job (D-36 / braces D-44), not the predicate's. A genuinely heterogeneous
  metadata OR (rare) → multiple queries or `defer-to-client` (D-58).
  - **Leaf vocabulary:** **name** (exact / single-segment glob / substring /
    extension / `∈ set`), **entry type** (file/dir/other), **reparse** (bool +
    tag; the D-52 follow policy), **attribute bitmask** (mask + expected set/clear;
    Windows-rich, Linux-sparse), **size** (compare/range), **timestamps**
    (btime/mtime/atime/ctime compare), **depth** (compare). Owner/mode/ACL and
    hardlink count are **out of v1** → `defer-to-client`.
  - Evaluated **inline** on the worker (no client code on engine threads, no
    round-trip) — this preserves the core value, free inline metadata pruning
    (D-13). Composes with the glob set: descend = `(any pattern wants it) AND
    descend-expr`; emit = `(some pattern matches) AND emit-expr`.
- **D-58. Optional `defer-to-client` ring escalation.** For conditions the
  declarative vocabulary cannot express, a predicate may yield **`defer-to-client`**:
  the engine posts a **decision-request** on the CQ (entry metadata + correlation
  token), the client's logic runs **on the client's own thread**, and the client
  submits the decision on the SQ; the parked scan resumes on arrival — reusing the
  D-11 suspension machinery. Costs the declarative path lacks: a **client liveness
  contract** (must drain and answer outstanding decision-requests or deadlock) and
  **critical-path latency / a throughput ceiling** if overused. Guidance:
  declarative for the bulk, escalate sparingly. A convenience adapter may accept a
  plain client `Fn(&Entry) -> Decision`, run it on a client-owned thread, and hide
  the SQ/CQ plumbing (closure DX without engine-thread contamination).
- **D-59. Unifying principle: every wait is a queue-resumed continuation
  suspension.** Enumeration I/O (`TP_IO`), output backpressure (D-11),
  `defer-to-client` (D-58), and cancellation teardown (D-50) are all the *same*
  mechanism — a parked continuation resumed by a queue event. This uniformity is
  intentional and load-bearing; do **not** later special-case any of the four into
  a bespoke path.
- **D-60. Ring servicing = waitable primitive + non-blocking drain, with
  adapters.** The response ring exposes a **waitable handle** (Windows event
  `HANDLE` / Linux eventfd) signalling "non-empty" plus a non-blocking `drain()`;
  blocking-wait, poll, foreign-reactor registration, and async `Stream`/`Waker`
  are all **thin adapters** over that pair. Who services the ring is flexible and
  may change at runtime. Backpressure closes the loop: the ring is **bounded, no
  drops** — when full the engine suspends continuations (D-11); the client's drain
  rate is the throttle. (Resolves O-C.)
- **D-61. Cancellation = an SQ submission, acked by a terminal CQ marker.** Cancel
  is a small **submission** (io_uring `ASYNC_CANCEL`-style), **not** a ring-level
  flag or out-of-band bit. It triggers the D-50 drain and is acknowledged by a
  **terminal CQ marker that lands *after* all CQ items already enqueued** (FIFO) —
  cancellation is *drain-what's-queued-then-terminal*, not stop-and-discard; a
  client wanting discard simply stops draining and drops. **Dropping the query
  handle** is the RAII convenience (submit-cancel + block on the terminal marker
  in `Drop`), giving async teardown a real join point. (Resolves O-D.)
- **D-71. Fatal errors terminate the walk with `Terminal{Failed}` (refines D-53,
  extends D-61).** Most failures stay per-entry / per-container error items and the
  walk continues (D-53). A **fatal** error instead stops the whole enumeration and is
  acknowledged by a `Terminal{Failed}` marker. The causing error is **not** carried
  *inside* the terminal (the reason stays a `Copy` unit value) — it rides in the
  `CqItem::Error` emitted immediately before the terminal. **Initial fatal policy = a
  root (depth-0) directory that cannot be enumerated at all is fatal**; every failure
  below the root remains per-container / per-entry and the walk continues. Recorded
  open point: with multiple roots this terminates the whole query on the first
  unopenable root — whether a bad root should instead be a per-root error item while
  sibling roots continue is left for a future refinement (raise before relying on it).
- **D-72. Reparse/symlink follow is a client `FollowLinks` policy (realizes D-52).**
  `Options.follow_links` is `FollowLinks::{Never, Always}`, default **`Never`** — the
  library never auto-follows (D-13/D-52). The engine descend gate treats a reparse
  entry (symlink / junction) as a directory *candidate* only under `Always`, on both
  platforms; a plain non-reparse directory is always a candidate. Once a candidate,
  the existing filters apply unchanged: pattern viability (`should_descend`, D-39),
  the client's `descend` conjunction (D-56/D-66), and the D-51 cycle guard — so
  following is *just filtering* plus the one default toggle a conjunction cannot
  express (an AND can only narrow, never opt back in). A followed target that is not
  actually a directory fails to enumerate and surfaces as a per-entry `CqItem::Error`
  (D-53); we do not stat-through to pre-classify. Loop safety under `Always` needs no
  target identity: any loop re-traverses a reparse point whose own file-id repeats and
  is deduped by the D-51 visited set on the second encounter. **Behavior change:** the
  native Windows backend previously auto-followed directory symlinks (they classify as
  `Dir`+reparse); it now defaults to `Never` too, so both platforms are consistent.
  `FollowLinks` is an enum, not a bool, to leave room for a future policy (e.g.
  same-volume-only) without an ABI break.
- **D-69. Ring implementation choices (M6).** The ring/API surface is realized with
  three v1 simplifications, each an owned decision (not a delegation) with a named
  reason and a deferral gated on a real factor, not on "no consumer":
  - **Name blob = reversible code-point form, not raw native units.** A `Match` /
    `ContainerEnter` / `DecisionRequest` carries the entry name as the crate's
    decoded code-point sequence (D-46), because the entire matcher pipeline and
    `DirEntry` already operate in code-point space and the transform is lossless —
    this *is* D-63's "one representation, two readers", just realized as the decoded
    form rather than the native `[u16]`/`[u8]`. The true zero-copy native blob in a
    512-byte inline descriptor (D-68) is deferred behind profiling, not lack of
    need.
  - **`PatternMask` is word-growable, not a fixed `u64`.** A `Match` bitset is a
    `Box<[u64]>` sized to the query's pattern count, so a query is never silently
    capped at 64 patterns (D-40). Owned items already heap-allocate their name
    (D-68), so the extra allocation is consistent; a fixed-inline bitset is a later
    optimization.
  - **Waitable primitive = portable `Signal` (Condvar), raw OS handle deferred.**
    Ring readiness and producer backpressure use the portable [`sys::signal::Signal`]
    (D-60), mirroring the portable-first enumeration backend. The raw
    `HANDLE`/`RawFd` exposure for foreign reactors is a native follow-up, blocked on
    the same native-servicing work as the async engine (M7), not on a missing
    consumer. `push_blocking` is documented as single-producer/test-suited; the
    engine uses `try_push` + its own unified suspension (D-59) for multi-producer
    backpressure.
- **D-70. The M7 engine is a synchronous worker pool realizing Model B's contract;
  the async orchestration is a later, behavior-preserving swap.** The engine
  (`engine.rs`) is a pool of `permits` worker threads (D-8: one permit = one live
  scan), each pulling one directory-scan job (D-7), enumerating it **synchronously**
  via `sys::enumerate` (native backend where present — D-4/D-6, which explicitly
  makes Linux enumeration a blocking pool-thread syscall), evaluating emit/descend
  **inline** (D-56), emitting CQ items, and pushing child directories back onto a
  LIFO work stack (depth-first bias, D-48). This is a faithful realization of the
  observable contract, **owned by us** (we define the behavior; the async backend
  is chosen later to satisfy the same contract), with these specifics:
  - **Container Enter/End (D-64) via a refcount cascade (D-9).** Each container's
    refcount = 1 (own scan) + one per launched child; `release` decrements and
    cascades upward under the state lock, collecting the zero-hitting containers,
    then emits their `ContainerEnd`s **outside** the lock (never hold the state lock
    across a ring push). This yields bottom-up ends and enter-before-children by
    construction. **The 1:1 enter/end guarantee holds even under cancellation/fatal
    error:** a container is tracked as *open* the moment its `ContainerEnter` is
    emitted; ends are emitted cancel-immune (`push_blocking`) and idempotently (guarded
    by the open-set), and after the workers join the coordinator closes any container
    still open — including a subtree the cancel abandoned before its refcount reached
    zero — bottom-up, *before* the terminal. So a client never reaches the terminal
    with a live container.
  - **Unified suspension (D-59) = worker-thread blocking.** Both I/O wait (the
    enumeration syscall) and output backpressure (a full ring) suspend the *worker
    thread* — the thread *is* the continuation in the sync model. Backpressure uses
    a cancel-responsive timed wait (`wait_space_timeout`) so a parked producer
    re-checks the cancel flag; the client's drain rate is the throttle (D-11).
  - **Cancellation (D-61) + accounting (D-50).** An `outstanding` job counter drives
    normal completion (0 ⇒ done); a cancel flag (set via `SqOp::Cancel` on the SQ,
    or immediately by `EngineHandle::drop`) makes workers stop pulling and bail
    their emits. A coordinator thread joins the workers then pushes the single
    `Terminal{Completed|Cancelled|Failed}` last (FIFO; `Failed` = fatal error, D-71).
    `EngineHandle::drop` is the RAII
    teardown: cancel, drain to unblock any parked terminal push, then join — so a
    client that drops without draining never deadlocks.
  - **Cycle detection (D-51).** A reparse-point directory is descended at most once
    per `(volume, file-id)` (a shared visited set); non-reparse dirs and unknown
    ids (portable-Windows zero) pass through.
  - **Native names.** `sys::encode_os_name` is the exact reverse of the D-46 decode,
    used to rebuild a child's physical path from its decoded code points.
  - **Deferred, each with a named technical blocker (not "no consumer"):**
    - *Relative-open (openat / handle-relative NtCreateFile, D-9).* We queue paths
      (D-10) and open each child by full path; true parent-fd-relative open needs
      the enumeration backend to accept a parent handle, which couples with the
      native async backends (M7-6). → CHECKLIST M7-6.
    - *`defer-to-client` escalation (D-58).* Needs a **tri-state** predicate leaf
      (accept / reject / defer) the M4 bool vocabulary lacks; the SQ
      `DecisionAnswer` plumbing and park/resume mechanism are ready to receive it.
      → CHECKLIST M7-7.
    - *Syscall-level FS-filter pushdown at terminal literal segments (D-19).* The
      sound *descend* pruning (viability) is done; pushing a name filter into the
      enumeration syscall is a pure performance add. → CHECKLIST M∞-2.

---

## 9. Explicit non-goals (for now)

- **D-43.** Out of scope: Alternate Data Streams (`file:stream`); Win32 reserved
  device names (`CON`, `NUL`, …); Win32 trailing dot/space munging (handled
  ordinally, D-30); pattern-complexity / DoS caps (self-inflicted, not our
  concern).

---

## 10. Open questions (NOT yet decided)

Conceptual forks are closed; what remains is schema / implementation-level detail.

- **O-C′. Ring concrete schema (in progress).** Payload model **settled** (D-63)
  and container model **settled** (D-64), ABI target **settled** (D-65:
  Rust-native), query-def + SQ op set + compilation **settled** (D-66; #5/#4), CQ
  ring + owned-vs-borrowed **settled** (D-68). Still open: only the **over-cap name
  spill mechanism** (D-63) — spill-to-side vs error item.
- **O-D′. Cancellation accounting mechanism** (D-50): the exact outstanding-work
  counters and the single teardown/drain path. (Model settled; counters TBD.)
- **O-E. `sys`-layer concrete surface** per platform (D-54): which primitives are
  wrapped and the shape of the safe wrappers.

### Resolved this session
- **O-B′. Declarative predicate vocabulary** (D-56 / D-14 / D-62): two AND-only
  predicates (emit, descend; prune = descend-false), signed + set-membership
  leaves, fixed leaf vocabulary, lazy-`statx`. Only the literal Rust enum shapes
  remain as ordinary implementation detail (fold into O-C′).

### Deferred (decided-to-defer, not open design questions)
- Full `[...]` character classes → a future dialect version (`@2`), D-44 / O-6.
- macOS backend (`getattrlistbulk`) → not a priority, D-15 / O-15.
- Unicode normalization → revisit before 1.0, D-27.
- Exclusion / negative patterns → door open, D-42.
