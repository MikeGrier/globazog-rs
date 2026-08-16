// Copyright (c) 2026 Mike Grier

//! Unit tests for the D-75 confinement containment primitives (`canon_path_key` /
//! `within`), independent of the filesystem.

use super::{canon_path_key, within};
use std::path::Path;

#[test]
fn within_is_component_wise_not_string_prefix() {
    // A string-prefix sibling (`rootsibling` vs `root`) is not contained.
    let root = canon_path_key(Path::new("/base/root"));
    let sib = canon_path_key(Path::new("/base/rootsibling/x"));
    let child = canon_path_key(Path::new("/base/root/x"));
    assert!(within(&root, &child));
    assert!(!within(&root, &sib));
    assert!(within(&root, &root));
}

// Regression for the case-fold escape (D-75): on a per-directory case-sensitive tree
// (NTFS-with-case-sensitivity / WSL-managed dirs) `Foo` and `foo` are genuinely
// distinct directories. `std::fs::canonicalize` returns each in its true on-disk
// casing, so an exact component compare must keep them distinct — a fold would collapse
// them and let a reparse point targeting `foo` escape a root at `Foo`.
#[cfg(windows)]
#[test]
fn windows_case_only_siblings_are_not_contained() {
    let foo_root = canon_path_key(Path::new(r"C:\base\Foo"));
    let foo_lower = canon_path_key(Path::new(r"C:\base\foo\secret"));
    assert_ne!(foo_root, canon_path_key(Path::new(r"C:\base\foo")));
    assert!(
        !within(&foo_root, &foo_lower),
        "case-only sibling escaped confinement: fold must not be applied"
    );
    // The same-cased child is still contained.
    let foo_child = canon_path_key(Path::new(r"C:\base\Foo\secret"));
    assert!(within(&foo_root, &foo_child));
}
