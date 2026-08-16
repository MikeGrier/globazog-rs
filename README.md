<!-- Copyright (c) 2026 Mike Grier -->
# globazog-rs

[![CI](https://github.com/MikeGrier/globazog-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MikeGrier/globazog-rs/actions/workflows/ci.yml)
[![release-please](https://github.com/MikeGrier/globazog-rs/actions/workflows/release-please.yml/badge.svg)](https://github.com/MikeGrier/globazog-rs/actions/workflows/release-please.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A high-performance, **Windows-first** globbing and directory-traversal library for
Rust: bounded parallel scanning with metadata-aware filter-before-recurse, an
owned multi-dialect glob syntax, and an io_uring-shaped submit/complete API.

## Crate

| Crate | What it is |
|---|---|
| [`globazog`](crates/globazog) | The globbing / directory-traversal library. |

## Design

The full design (decisions `D-1`…`D-73`) lives in
[DESIGN-NOTES.md](DESIGN-NOTES.md); the build plan is in
[CHECKLIST.md](CHECKLIST.md).

## Build

Requires a recent Rust toolchain (MSRV: see `[workspace.package].rust-version`
in [Cargo.toml](Cargo.toml)).

```powershell
cargo build --workspace --release
cargo test --workspace
```

## Release

Versioning and tagging are automated with
[`release-please`](.github/workflows/release-please.yml) using
[Conventional Commits](https://www.conventionalcommits.org/); merging the
Release PR tags a `v<version>` release, which drives publishing to crates.io.

## License

MIT — see [LICENSE](LICENSE).
