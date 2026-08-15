// Copyright (c) 2026 Mike Grier

//! A minimal end-to-end globber (D-57): walk a directory with two patterns and a
//! per-pattern emit filter, printing each match (with the size and which patterns
//! matched) and the terminal outcome.
//!
//! Run with: `cargo run --example glob -- <dir>` (defaults to the current dir).

use globazog::{Cmp, CqItem, Dialect, Leaf, QueryBuilder, TerminalReason};

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());

    let handle = QueryBuilder::new()
        .root(&dir)
        // Pattern 0: every Rust source file, recursively.
        .pattern("**/*.rs", Dialect::Posix, Vec::new())
        // Pattern 1: non-empty markdown files (a per-pattern emit filter, D-66).
        .pattern(
            "**/*.md",
            Dialect::Posix,
            vec![Leaf::Size {
                op: Cmp::Gt,
                value: 0,
            }],
        )
        .submit()
        .expect("valid query");

    let ring = handle.completions();
    let mut matches = 0usize;
    loop {
        match ring.wait_pop() {
            CqItem::Match(m) => {
                matches += 1;
                let which: Vec<usize> = m.matched.iter().collect();
                println!(
                    "{:>10} bytes  patterns={:?}  {}",
                    m.meta.size,
                    which,
                    m.name.to_string_lossy()
                );
            }
            CqItem::Error(e) => eprintln!("error: {}", e.error),
            CqItem::Terminal(t) => {
                let outcome = match t.reason {
                    TerminalReason::Completed => "completed",
                    TerminalReason::Cancelled => "cancelled",
                };
                println!("{outcome}: {matches} match(es)");
                break;
            }
            _ => {}
        }
    }
}
