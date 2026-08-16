<!-- Copyright (c) 2026 Mike Grier -->
# globazog — Design Rationale (Tier 2)

Historical record of *how and why* decisions were reached — alternatives considered,
tradeoffs, and evolutionary reasoning — cross-referenced by decision ID from the
current-decisions record in [DESIGN-NOTES.md](DESIGN-NOTES.md) (Tier 1). Tier 1 is
authoritative for *what* the current contract is; this file explains *why*. If the two
ever disagree, Tier 1 wins.

This file is bootstrapped with the newest decision that carries substantive tradeoffs;
older decisions' rationale still lives inline in [DESIGN-NOTES.md](DESIGN-NOTES.md) until
migrated.

## D-75. Root confinement against reparse-point escape

**Contract:** see [DESIGN-NOTES.md](DESIGN-NOTES.md) → D-75.

### Alternatives considered

- **Containment mechanism — canonicalize-and-compare vs file identity.** Chosen:
  canonicalize each candidate target and each root (`std::fs::canonicalize`) and compare
  components **exactly** (no case fold). Rejected: keying on the `(volume, file-id)`
  identity the engine already fetches for cycle detection (D-51) — identity answers
  "same object," not "inside the roots' *path* subtree," and a target on a different
  volume has no identity relationship to a root at all. Path containment is the actual
  question, so a path primitive is the right tool.

- **Exact component compare vs case fold.** Chosen: compare canonical components
  **exactly**. An earlier revision folded components on Windows (D-28) to be
  case-insensitive, but that is wrong: Windows now supports **per-directory case
  sensitivity** (NTFS, common under WSL-managed trees), where `Foo` and `foo` are
  genuinely distinct directories. Folding collapses them, so a junction targeting `foo`
  would be judged contained by a root at `Foo` — an actual escape. The fold is also
  unnecessary: `canonicalize` (Windows `GetFinalPathNameByHandle`) already returns each
  component in its true on-disk casing, so on a case-insensitive volume the root and the
  target resolve to the *same* stored casing and compare equal without folding. Exact
  compare is therefore correct on both filesystem kinds; the fold was only ever correct
  by accident on the case-insensitive default. Regression:
  `engine::tests::windows_case_only_siblings_are_not_contained`.


- **Fail-open vs fail-closed on an unresolvable target.** Chosen: **fail-closed** — a
  target that cannot be canonicalized (broken / inaccessible) is declined as
  `RootEscape`. Rationale: a *bounding* feature must never follow a reparse point it
  cannot prove stays inside; the cost is that a broken or permission-blocked in-root link
  is also declined (and reported as `RootEscape`), which is acceptable for the bounding
  use. Fail-open (fall through to the normal enumerate, which would then error) was
  rejected: it weakens the guarantee for a marginal accuracy gain, and it is unsafe to
  argue "canonicalize failure implies enumerate failure" in general (path-permission
  cases can differ).

- **Cut-off vs client override.** Chosen: **cut-off only**, emitting an informational
  `CqItem::Blocked { RootEscape }`. Rationale: the stated use is roots-as-a-bound, which
  just wants the descent stopped. A client override (allow specific escapes) would need
  the D-58 defer-to-client decision machinery (park the scan, await an `SqOp` answer),
  which is deferred (M7-7). The door is left open cheaply by making `BlockReason`
  `#[non_exhaustive]` and keeping `Blocked` a structured item that could later carry a
  decision token.

- **Where to check.** Chosen: in the descend gate, only for reparse candidates (which
  only occur under `FollowLinks::Always`), so the feature is inert under `Never` and off
  by default. Roots are canonicalized once at engine spawn (and only when confinement is
  actually active) rather than per-entry.

### Known limitation — TOCTOU

The check canonicalizes the link path, but the synchronous backend's full-path open
(M5-4) re-resolves that path when it enumerates the target *later*. Between the two, an
adversary who swaps the link or a mutable ancestor can still escape. Confinement is thus
a best-effort bound against misconfiguration / accidental escape, **not** an
adversary-hardened boundary. Closing the race requires handle-relative /
`openat`-no-follow resolution so the containment check and the enumeration act on the
*same* opened object — deferred to the M7-6 relative-open work, which already has to
replace the full-path open for parent-relative traversal.
