// Copyright (c) 2026 Mike Grier

//! Platform layer: sharp `unsafe` per-OS enumeration APIs behind safe wrappers
//! (D-54) — Windows IOCP / `NtQueryDirectoryFile` / `CreateThreadpoolIo`, Linux
//! io_uring / `getdents64` / `openat` / `statx` — unified behind one
//! completion-based enumeration abstraction (D-5, D-6).
