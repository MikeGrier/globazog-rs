// Copyright (c) 2026 Mike Grier

//! A minimal end-to-end globber (D-57): walk a directory with one or more patterns
//! and report per-pattern match counts, a small sample, and the terminal outcome.
//!
//! Usage: `cargo run --release --example glob -- <dir> [pattern...]`
//! (defaults to the current dir and `**/*.rs` + `**/*.md`). Patterns use the `win`
//! dialect on Windows (case-insensitive) and `posix` elsewhere.

use globazog::{CqItem, Dialect, QueryBuilder, TerminalReason};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| ".".to_string());
    let mut patterns: Vec<String> = args.collect();
    if patterns.is_empty() {
        patterns = vec!["**/*.rs".to_string(), "**/*.md".to_string()];
    }

    let dialect = if cfg!(windows) {
        Dialect::Win
    } else {
        Dialect::Posix
    };

    let mut builder = QueryBuilder::new().root(&dir);
    for p in &patterns {
        builder = builder.pattern(p, dialect, Vec::new());
    }
    let handle = builder.submit().expect("valid query");

    let ring = handle.completions();
    let started = Instant::now();
    let mut per_pattern = vec![0usize; patterns.len()];
    let mut total = 0usize;
    let mut total_bytes = 0u64;
    let mut errors = 0usize;
    let mut dirs = 0usize;
    let mut sample: Vec<String> = Vec::new();

    loop {
        match ring.wait_pop() {
            CqItem::Match(m) => {
                total += 1;
                total_bytes += m.meta.size;
                for i in m.matched.iter() {
                    per_pattern[i] += 1;
                }
                if sample.len() < 20 {
                    sample.push(format!("{:>12}  {}", m.meta.size, m.name.to_string_lossy()));
                }
            }
            CqItem::ContainerEnter(_) => dirs += 1,
            CqItem::Error(_) => errors += 1,
            CqItem::Terminal(t) => {
                let elapsed = started.elapsed();
                println!("\n--- sample (first {} matches) ---", sample.len());
                for line in &sample {
                    println!("{line}");
                }
                println!("\n--- per-pattern counts ---");
                for (p, count) in patterns.iter().zip(&per_pattern) {
                    println!("{count:>10}  {p}");
                }
                let outcome = match t.reason {
                    TerminalReason::Completed => "completed",
                    TerminalReason::Cancelled => "cancelled",
                    TerminalReason::Failed => "failed",
                };
                println!(
                    "\n{outcome}: {total} matches, {total_bytes} bytes, {dirs} dirs scanned, \
                     {errors} errors, in {:.2}s",
                    elapsed.as_secs_f64()
                );
                break;
            }
            _ => {}
        }
    }
}
