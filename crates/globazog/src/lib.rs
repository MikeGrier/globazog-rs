// Copyright (c) 2026 Mike Grier

//! `globazog` — a high-performance, Windows-first globbing and directory-traversal
//! library.
//!
//! The design is recorded in `DESIGN-NOTES.md` at the repository root; each module's
//! docs cite the relevant decision IDs (`D-n`).

pub mod builder;
pub mod error;
pub mod predicate;
pub mod ring;
pub mod syntax;

mod engine;
mod sys;
