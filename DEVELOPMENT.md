<!-- Copyright (c) 2026 Mike Grier -->

# Development & session handoff

Durable notes for continuing work on this repository (the crate is `globazog`).
This file exists so context survives a fresh clone; the authoritative design and
plan live in [DESIGN-NOTES.md](DESIGN-NOTES.md), [CHECKLIST.md](CHECKLIST.md),
[COMPLETED-CHECKLIST.md](COMPLETED-CHECKLIST.md), and [PLANS.md](PLANS.md).

## Current status (2026-08-14)

- **Numbered milestones M1–M8 are complete**: matcher → dialects → predicates →
  `sys` (native Windows `NtQueryDirectoryFile` + Linux `openat`/`getdents64`/`statx`
  backends, portable `std::fs` fallback) → ring (SQ/CQ) → the synchronous Model B
  engine → end-to-end integration, an example, and docs.
- The library is working end-to-end: `examples/glob.rs` scans a ~43k-directory
  drive for `**/*.h` in ~1.3 s (release).
- Test counts: ~129 on Windows, ~131 on Linux (extra unix-gated tests), all green;
  `cargo doc` is warning-free.
- Edition **2024**, MSRV **1.97** (CI MSRV job pinned to 1.97.0).

### Remaining work (all gated on real blockers, tracked in [CHECKLIST.md](CHECKLIST.md))

- **M7-6** — native async backends (Windows IOCP/`TP_IO`/`TP_WORK`, Linux io_uring),
  parent-fd relative-open (openat / handle-relative `NtCreateFile`), and restoring
  the `\\?\` NT-layer open for Windows long paths (>260) / trailing-dot-space names
  (also carries the M8-2 integration coverage for those, which cannot be created
  through the Win32 layer). Gated on building the D-5 completion abstraction + async
  FFI.
- **M7-7** — `defer-to-client` predicate escalation (D-58). Gated on adding a
  **tri-state** predicate leaf (accept/reject/defer); the SQ `DecisionAnswer`
  plumbing and park/resume are already in place.
- **M∞-1 / M∞-2** — zero-copy inline name blob + over-cap spill; syscall-level
  FS-filter pushdown at terminal literal segments. Gated on profiling.

The engine's model is decision **D-70** in [DESIGN-NOTES.md](DESIGN-NOTES.md).

## Build & test — Windows (host)

Use the **cargo-mcp** tools (never terminal `cargo`) and the **tpu-mcp** tools for
file I/O (LF-only repo), per [.github/copilot-instructions.md](.github/copilot-instructions.md).
Always pass the workspace root as `working_dir`.

Milestone gate (both profiles, zero warnings): `cargo fmt`, `cargo clippy
--all-targets`, `cargo test`, then `cargo check --all-targets --release`. Do **not**
add `--workspace` (respect `default-members`).

## Build & test — Linux (via WSL)

A WSL2 Ubuntu distro is set up on this machine and is the Linux dev/CI-iteration
environment (it unblocked the native Linux backend, M5-5). Key facts:

- **Toolchain**: `rustup` installed for the WSL user, `stable` (rustc 1.97.1) with
  the `clippy` and `rustfmt` components. Apt prereqs installed as root:
  `curl build-essential pkg-config ca-certificates`.
- **Isolated target dir**: always set `CARGO_TARGET_DIR=$HOME/globazog-target` so the
  Linux build does not fight the Windows `target/` on the shared drive.
- **Repo mount**: the Windows drive is mounted at `/mnt/<drive>`, so this repo is at
  `/mnt/q/github/<repo-folder>` (adjust `<repo-folder>` after the rename/re-clone).

Run the suite (replace the user and path as needed):

```pwsh
wsl -u <user> -e bash -lc 'source $HOME/.cargo/env; cd /mnt/q/github/<repo-folder>; \
  export CARGO_TARGET_DIR=$HOME/globazog-target; \
  cargo fmt --check && cargo clippy --all-targets && cargo test'
```

Install steps, if a fresh WSL environment is ever needed again:

```pwsh
wsl -u root -e bash -lc 'apt-get update && DEBIAN_FRONTEND=noninteractive \
  apt-get install -y curl build-essential pkg-config ca-certificates'
wsl -u <user> -e bash -lc 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --default-toolchain stable --profile minimal'
wsl -u <user> -e bash -lc 'source $HOME/.cargo/env; rustup component add clippy rustfmt'
```

## Release / publish to crates.io

Wired but **not yet live** (see the workflow files):

- [.github/workflows/release-please.yml](.github/workflows/release-please.yml) opens
  a Release PR on push to `main`.
- [.github/workflows/publish-crate.yml](.github/workflows/publish-crate.yml) runs
  `cargo publish -p globazog --locked` on a `v*` tag.

Gating factors before a real publish:

1. **Secrets (owner-only)**: `RELEASE_PLEASE_TOKEN` (PAT, `repo` scope — required so
   the release tag triggers the publish workflow) and `CARGO_REGISTRY_TOKEN`
   (crates.io token).
2. **Runs only on `main`** — work must be merged there.
3. **Conventional Commits** — release-please derives the bump from `feat:` / `fix:`
   messages; the current history uses `Completed item:` style, so a merge would open
   no Release PR until a conventional commit lands.
4. `globazog` must be available/owned on crates.io (first publish claims the name).

## Post-rename follow-up

The repository is being renamed to match the crate (`globazog`). After the rename,
update the `repository` / `homepage` URLs in the root [Cargo.toml](Cargo.toml)
(currently `github.com/MikeGrier/globbinobulous-rs`) and any doc cross-links to the
new name. GitHub redirects keep the old URLs working in the meantime, so this is not
urgent.
