// Copyright (c) 2026 Mike Grier

//! A minimal end-to-end globber (D-57): walk a directory with one or more patterns
//! and report per-pattern match counts, a small sample, and the terminal outcome.
//!
//! Usage: `cargo run --release --example glob -- <dir> [pattern...]`
//! (defaults to the current dir and `**/*.rs` + `**/*.md`). Patterns use the `win`
//! dialect on Windows (case-insensitive) and `posix` elsewhere.

use globazog::{CqItem, Dialect, MetaMask, QueryBuilder, TerminalReason};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir_arg = args.next().unwrap_or_else(|| ".".to_string());
    // Roots must be absolute (D-74); resolve the CLI argument against one CWD snapshot
    // (an absolute argument replaces the snapshot).
    let dir = std::env::current_dir().expect("current dir").join(&dir_arg);
    let mut patterns: Vec<String> = args.collect();
    if patterns.is_empty() {
        patterns = vec!["**/*.rs".to_string(), "**/*.md".to_string()];
    }

    let dialect = if cfg!(windows) {
        Dialect::Win
    } else {
        Dialect::Posix
    };

    // The report prints each match's byte size, so request SIZE explicitly (D-62):
    // stat-tier fields are only populated when a predicate or the result shape asks.
    let mut builder = QueryBuilder::new().root(&dir).result_shape(MetaMask::SIZE);
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
                let outcome = match t.reason {
                    TerminalReason::Completed => "completed",
                    TerminalReason::Cancelled => "cancelled",
                    TerminalReason::Failed => "failed",
                };
                let report = Report {
                    patterns: &patterns,
                    per_pattern: &per_pattern,
                    sample: &sample,
                    total,
                    total_bytes,
                    dirs,
                    errors,
                    outcome,
                    elapsed_secs: started.elapsed().as_secs_f64(),
                };
                let mut out = String::new();
                report.render(&mut out);
                print!("{out}");
                break;
            }
            _ => {}
        }
    }
}

/// One run's results, rendered independently of the destination (D-57). `render`
/// writes to any text sink, so the report format is separable from stdout — the
/// example emits it from a single site and it can be captured (e.g. into a `String`).
struct Report<'a> {
    patterns: &'a [String],
    per_pattern: &'a [usize],
    sample: &'a [String],
    total: usize,
    total_bytes: u64,
    dirs: usize,
    errors: usize,
    outcome: &'a str,
    elapsed_secs: f64,
}

impl Report<'_> {
    fn render(&self, out: &mut impl std::fmt::Write) {
        let _ = writeln!(
            out,
            "\n--- sample (first {} matches) ---",
            self.sample.len()
        );
        for line in self.sample {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "\n--- per-pattern counts ---");
        for (p, count) in self.patterns.iter().zip(self.per_pattern) {
            let _ = writeln!(out, "{count:>10}  {p}");
        }
        let _ = writeln!(
            out,
            "\n{}: {} matches, {} bytes, {} dirs scanned, {} errors, in {:.2}s",
            self.outcome, self.total, self.total_bytes, self.dirs, self.errors, self.elapsed_secs
        );
    }
}
