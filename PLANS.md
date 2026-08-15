# Plans

Tracks all [CHECKLIST.md](CHECKLIST.md) files in this component and their status.

| Path to CHECKLIST.md | Status | Brief description | Design Notes |
|---|---|---|---|
| [CHECKLIST.md](CHECKLIST.md) | in progress | Build the `globazog` globbing/traversal library bottom-up. Completed milestones M1–M10 (matcher → dialects → predicates → `sys` incl. native Win/Linux backends → ring → synchronous engine → end-to-end integration, example, docs → per-entry error propagation + fatal-error terminal, D-71 → client-controlled symlink follow policy, D-72) are archived in [COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md). Gated/horizon follow-ups remain in CHECKLIST.md: M7-6 (native async IOCP/io_uring + relative-open + `\?\` long paths), M7-7 (`defer-to-client`, needs a tri-state predicate), M∞-1/M∞-2 (zero-copy name blob, FS-filter pushdown). | [DESIGN-NOTES.md](DESIGN-NOTES.md) |
