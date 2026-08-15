# Plans

Tracks all [CHECKLIST.md](CHECKLIST.md) files in this component and their status.

| Path to CHECKLIST.md | Status | Brief description | Design Notes |
|---|---|---|---|
| [CHECKLIST.md](CHECKLIST.md) | in progress | Build the `globazog` globbing/traversal library bottom-up. Milestones M1–M8 complete (matcher → dialects → predicates → `sys` incl. native Win/Linux backends → ring → synchronous engine → end-to-end integration, example, docs). M9 (review-driven per-entry error propagation + fatal-error terminal, D-71) is complete. Gated/horizon follow-ups remain: M7-6 (native async IOCP/io_uring + relative-open + `\?\` long paths), M7-7 (`defer-to-client`, needs a tri-state predicate), M∞-1/M∞-2 (zero-copy name blob, FS-filter pushdown). | [DESIGN-NOTES.md](DESIGN-NOTES.md) |
