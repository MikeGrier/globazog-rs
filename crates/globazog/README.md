<!-- Copyright (c) 2026 Mike Grier -->

# globazog

A high-performance, Windows-first globbing and directory-traversal library for Rust.

`globazog` answers a **set of glob patterns over a set of roots in a single
traversal**. It walks directories on a permit-bounded pool of background threads,
matches and filters entries *inline* using the filesystem's own inline metadata
(reparse bit, size, timestamps), and streams results back to you through a bounded,
no-drop **completion ring**. Native enumeration backends
(`NtQueryDirectoryFile` on Windows, `getdents64` + `statx` on Linux) are used where
available, with a portable `std::fs` fallback everywhere else.

On a real machine, a full scan of a ~43,000-directory drive for `**/*.h` returns
~39,000 matches in ~1.3 s (release).

## Features

- **Multi-pattern, single traversal** — `*.c` + `*.h` + `Cargo.toml` are answered in
  one walk; each match reports *which* patterns it satisfied.
- **Owned glob dialects** — `posix` (`/` separators, `\` escapes, case-sensitive) and
  `win` (`/` and `\` separators, `{{`/`}}` brace escaping, case-insensitive), with
  `*`, `?`, `**` (whole-segment recursive), and `{a,b}` alternation.
- **Declarative predicates** — per-pattern *emit* filters and a per-query *descend*
  filter over name, type, size, timestamps, reparse status, attributes, and depth —
  evaluated inline, no callbacks on the engine threads.
- **Streaming completion ring** — matches, directory enter/end markers, per-entry
  errors, and a terminal marker arrive as owned items you drain at your own pace;
  the drain rate *is* the backpressure throttle. Nothing is dropped.
- **Cancellation** — cancel a running walk and receive a terminal marker after the
  already-queued items; dropping the handle tears the walk down cleanly.
- **Correct on messy filesystems** — reversible name decoding (unpaired UTF-16
  surrogates / non-UTF-8 bytes survive round-trip), reparse-cycle detection, and
  per-entry errors that never abort the walk.

## Quick start

```toml
[dependencies]
globazog = "0.1"
```

```rust,no_run
use globazog::{CqItem, Dialect, QueryBuilder};

// Every `*.rs` under the current directory, recursively.
let handle = QueryBuilder::new()
    .root(".")
    .pattern("**/*.rs", Dialect::Posix, Vec::new())
    .submit()
    .expect("valid query");

let ring = handle.completions();
loop {
    match ring.wait_pop() {
        CqItem::Match(m) => println!("{}", m.name.to_string_lossy()),
        CqItem::Terminal(_) => break,
        _ => {}
    }
}
```

## Concepts

### Query = roots × patterns

You build a query with [`QueryBuilder`]: add one or more **roots** (physical seed
directories), one or more **patterns**, then `submit()`. The pattern set is matched
against every root (the *pattern-is-gospel* model); an absolute pattern like
`/etc/*.conf` is lowered by peeling its literal prefix into a root. `submit()`
compiles the patterns (returning `Err` on a bad pattern) and starts the engine on
background threads, handing you a [`QueryHandle`].

### Dialects

Patterns are parsed in a named [`Dialect`]:

- [`Dialect::Posix`] — `/` separators, `\` escapes, UTF-8, **case-sensitive**.
- [`Dialect::Win`] — `/` **and** `\` separators (so `\` cannot escape), UTF-8,
  **case-insensitive**; escape a literal brace by doubling it (`{{` → `{`,
  `}}` → `}`). Only usable on Windows.

Metacharacters: `*` (any run within a segment), `?` (one code point), `**` (a whole
segment: zero-or-more directory levels), and `{a,b,c}` (alternation).

### Predicates: emit and descend

Beyond the glob, each entry can be filtered by declarative [`Leaf`] conditions
(ANDed together):

- **Emit** (per pattern) — the third argument to `.pattern(...)`. A match is reported
  only if the glob matches *and* every emit leaf holds. Empty = pass-all.
- **Descend** (per query) — `.descend(...)`. The engine recurses into a directory
  only if some pattern still wants it *and* every descend leaf holds.

```rust,no_run
use globazog::{Cmp, Dialect, Leaf, QueryBuilder};

let handle = QueryBuilder::new()
    .root(".")
    // Only emit logs larger than 1 KiB.
    .pattern("**/*.log", Dialect::Posix, vec![Leaf::Size { op: Cmp::Gt, value: 1024 }])
    // Never descend into reparse points (junctions / symlinked dirs).
    .descend(vec![Leaf::IsReparse { negate: true }])
    .submit()
    .expect("valid query");
```

Leaf constructors cover the common cases: [`Leaf::name_exact`], [`Leaf::name_glob`],
[`Leaf::name_contains`], [`Leaf::name_extension`], [`Leaf::name_in_set`], plus
`Size`, `Time`, `Depth`, `IsType`, `IsReparse`, `ReparseTag`, `AttrsAllSet`, and
`AttrsAllClear`.

> **Portability note:** `AttrsAllSet` / `AttrsAllClear` and `ReparseTag` carry
> Win32 semantics — the `FILE_ATTRIBUTE_*` bitmask and reparse tag are `0` on
> non-Windows, so those leaves don't match there. Use `IsReparse` / `IsType` for
> cross-platform type and symlink/reparse checks.

### The completion ring

`handle.completions()` is the [`CompletionRing`] you service. Pop items with
`wait_pop()` (blocking), `pop()` (non-blocking), or `drain()` (all queued). Each item
is a [`CqItem`]:

| Variant | Meaning |
|---|---|
| `Match(m)` | An entry matched. `m.name`, `m.matched` (a [`PatternMask`] — `m.matched.iter()` yields the pattern indices), and `m.meta` (size/timestamps/type). |
| `ContainerEnter(e)` | A directory scan started. `e.id`, `e.parent`, `e.name`. |
| `ContainerEnd(e)` | A directory subtree finished (bottom-up, 1:1 with its enter). |
| `Error(e)` | A per-entry/-subtree failure (e.g. permission denied); the walk continues past it. The one exception is a fatal error (see `Failed` below), which stops the walk. |
| `DecisionRequest(_)` | Reserved for `defer-to-client` (not yet emitted). |
| `Terminal(t)` | The walk ended. Always the **last** item. `t.reason` is `Completed`, `Cancelled`, or `Failed` — a fatal error (currently a root that could not be enumerated) stopped the walk; its causing `Error` item lands immediately before this terminal. |

Ordering guarantees you can rely on: a container's `ContainerEnter` precedes any
`Match`/child that references it, and `ContainerEnd`s cascade bottom-up (a child ends
before its parent). Sibling directories interleave freely. Full paths are not shipped
per entry — reconstruct them client-side by keeping a `container id → (parent, name)`
map from the `ContainerEnter` stream and walking to the root, mirroring the engine's
own parent chain.

### Options, backpressure, and cancellation

[`Options`] tunes `permits` (concurrent scans), `ring_capacity`, `cycle_detection`,
and a query-level case default. The ring is **bounded and never drops**: when it
fills, the engine parks its workers until you drain — so a slow consumer simply slows
the walk. Call `handle.cancel()` to stop early; you will still receive a terminal after
the already-queued items — `Terminal { reason: Cancelled }` when the request is honored
before the walk finishes, or `Completed` if the cancel raced with normal completion (a
`Completed` after `cancel()` is not a contract violation). Dropping the `QueryHandle`
cancels and joins the engine threads.

## Building a tool

[`examples/glob.rs`](examples/glob.rs) is a complete CLI built on this API —
multi-pattern, per-pattern emit filters, a per-pattern count summary, and a terminal
report. Run it with:

```sh
cargo run --release --example glob -- <dir> "**/*.c" "**/*.h"
```

## Platform support

- **Windows** — native `NtQueryDirectoryFile`-based enumeration with inline
  attributes, reparse tag, 128-bit file id, and all timestamps.
- **Linux** — native `openat` + `getdents64` + `statx` (birth time included).
- **Other** — a portable `std::fs` backend satisfying the same contract.

## Status

The core library (matcher, dialects, predicates, native/portable backends, ring, and
the synchronous scheduler) is complete and tested on Windows and Linux. Planned
follow-ups: native async backends (IOCP / io_uring) with parent-relative open and
`\\?\` long-path support, `defer-to-client` predicate escalation, and a zero-copy
name-blob fast path. See [CHECKLIST.md](../../CHECKLIST.md) and [DESIGN-NOTES.md](../../DESIGN-NOTES.md) at the repository root.

## License

MIT © Mike Grier

<!-- API reference targets (this is a package README, not rustdoc, so the code-span
     references above are made navigable with explicit docs.rs links). -->
[`QueryBuilder`]: https://docs.rs/globazog/latest/globazog/struct.QueryBuilder.html
[`QueryHandle`]: https://docs.rs/globazog/latest/globazog/struct.QueryHandle.html
[`Options`]: https://docs.rs/globazog/latest/globazog/struct.Options.html
[`CompletionRing`]: https://docs.rs/globazog/latest/globazog/struct.CompletionRing.html
[`PatternMask`]: https://docs.rs/globazog/latest/globazog/struct.PatternMask.html
[`CqItem`]: https://docs.rs/globazog/latest/globazog/enum.CqItem.html
[`Dialect`]: https://docs.rs/globazog/latest/globazog/enum.Dialect.html
[`Dialect::Posix`]: https://docs.rs/globazog/latest/globazog/enum.Dialect.html#variant.Posix
[`Dialect::Win`]: https://docs.rs/globazog/latest/globazog/enum.Dialect.html#variant.Win
[`Leaf`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html
[`Leaf::name_exact`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html#method.name_exact
[`Leaf::name_glob`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html#method.name_glob
[`Leaf::name_contains`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html#method.name_contains
[`Leaf::name_extension`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html#method.name_extension
[`Leaf::name_in_set`]: https://docs.rs/globazog/latest/globazog/enum.Leaf.html#method.name_in_set
