// Copyright (c) 2026 Mike Grier

//! M3-5 integration test: parse and match both dialects over a ~1,200-path corpus.

use globazog::syntax::dialect::Dialect;
use globazog::syntax::set::PatternSet;

fn cps(s: &str) -> Vec<u32> {
    s.chars().map(|c| c as u32).collect()
}

/// A deterministic corpus: 20 modules × 20 files, in `.rs` / `.txt` / `.md` trees.
fn corpus() -> Vec<Vec<Vec<u32>>> {
    let mut paths = Vec::new();
    for i in 0..20 {
        for j in 0..20 {
            paths.push(vec![
                cps("src"),
                cps(&format!("mod{i}")),
                cps(&format!("file{j}.rs")),
            ]);
            paths.push(vec![
                cps("src"),
                cps(&format!("mod{i}")),
                cps(&format!("file{j}.txt")),
            ]);
            paths.push(vec![
                cps("docs"),
                cps(&format!("guide{i}")),
                cps(&format!("page{j}.md")),
            ]);
        }
    }
    paths
}

fn count_matches(set: &PatternSet, paths: &[Vec<Vec<u32>>]) -> Vec<usize> {
    let mut counts = vec![0usize; set.len()];
    for path in paths {
        let refs: Vec<&[u32]> = path.iter().map(Vec::as_slice).collect();
        for idx in set.matches(&refs) {
            counts[idx] += 1;
        }
    }
    counts
}

#[test]
fn posix_set_over_corpus() {
    let paths = corpus();
    assert_eq!(paths.len(), 1200);

    let mut set = PatternSet::new();
    set.add("src/**/*.rs", Dialect::Posix, None).unwrap(); // 0
    set.add("**/*.txt", Dialect::Posix, None).unwrap(); // 1
    set.add("docs/**/*.md", Dialect::Posix, None).unwrap(); // 2
    set.add("src/mod0/*.rs", Dialect::Posix, None).unwrap(); // 3

    let counts = count_matches(&set, &paths);
    assert_eq!(counts[0], 400, "src/**/*.rs");
    assert_eq!(counts[1], 400, "**/*.txt");
    assert_eq!(counts[2], 400, "docs/**/*.md");
    assert_eq!(counts[3], 20, "src/mod0/*.rs");
}

#[test]
fn win_dialect_case_insensitive_over_corpus() {
    let paths = corpus();
    let mut set = PatternSet::new();
    // Uppercase, backslash-separated win pattern; case-insensitive by default so
    // it matches the lowercase corpus.
    set.add("SRC\\**\\*.RS", Dialect::Win, None).unwrap();

    let counts = count_matches(&set, &paths);
    assert_eq!(counts[0], 400);
}

#[test]
fn descend_pruning_over_corpus() {
    let mut set = PatternSet::new();
    set.add("src/**/*.rs", Dialect::Posix, None).unwrap();

    assert!(set.should_descend(&[]));
    assert!(set.should_descend(&[&cps("src")]));
    assert!(!set.should_descend(&[&cps("docs")]));
}
