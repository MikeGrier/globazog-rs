// Copyright (c) 2026 Mike Grier

//! The io_uring-shaped submit/complete API (D-55): the SQ op set (D-66), the CQ
//! item enum (Match / ContainerEnter / ContainerEnd / Error / DecisionRequest /
//! Terminal — D-64) on a bounded MPMC ring (D-68) with park/wake backpressure
//! (D-11) and a waitable-handle + `drain` servicing surface (D-60); cancellation
//! (D-61); Rust-native ABI (D-65).
