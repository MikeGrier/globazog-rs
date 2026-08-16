// Copyright (c) 2026 Mike Grier

use super::{Alphabet, Dialect};
use crate::syntax::CaseSensitivity;

#[test]
fn ids_and_defaults() {
    assert_eq!(Dialect::Posix.id(), "posix");
    assert_eq!(Dialect::Win.id(), "win");
    assert_eq!(Dialect::Posix.default_case(), CaseSensitivity::Sensitive);
    assert_eq!(Dialect::Win.default_case(), CaseSensitivity::Insensitive);
    assert_eq!(Dialect::Posix.alphabet(), Alphabet::Utf8);
}

#[test]
fn resolve_bare_id() {
    assert_eq!(Dialect::resolve("posix"), Some(Dialect::Posix));
    assert_eq!(Dialect::resolve("win"), Some(Dialect::Win));
    assert_eq!(Dialect::resolve("nope"), None);
}

#[test]
fn resolve_partial_versions_bind() {
    assert_eq!(Dialect::resolve("posix@1"), Some(Dialect::Posix));
    assert_eq!(Dialect::resolve("posix@1.0"), Some(Dialect::Posix));
    assert_eq!(Dialect::resolve("posix@1.0.0"), Some(Dialect::Posix));
    assert_eq!(Dialect::resolve("win@1"), Some(Dialect::Win));
}

#[test]
fn resolve_mismatched_versions_reject() {
    assert_eq!(Dialect::resolve("posix@2"), None);
    assert_eq!(Dialect::resolve("posix@1.1"), None);
    assert_eq!(Dialect::resolve("posix@1.0.1"), None);
    assert_eq!(Dialect::resolve("posix@x"), None);
    // Overlong (4+ components) and an empty suffix are not real versions.
    assert_eq!(Dialect::resolve("posix@1.0.0.0"), None);
    assert_eq!(Dialect::resolve("posix@"), None);
}
