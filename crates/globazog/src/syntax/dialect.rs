// Copyright (c) 2026 Mike Grier

//! Dialect registry (D-15–D-20): the closed set of named glob dialects, each with a
//! stable ASCII id, an alphabet flag, a default case-sensitivity, and `@`-based
//! version binding. New dialects are added to [`Dialect`].

use crate::syntax::CaseSensitivity;

#[cfg(test)]
mod tests;

/// The closed set of glob dialects (D-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// `/`-only separators, `\` escape, UTF-8, case-sensitive default (D-21).
    Posix,
    /// `/` and `\` separators, brace-doubling escape, UTF-8, case-insensitive
    /// default; Windows-only policy gate (D-22, D-47).
    Win,
}

/// Whether a dialect's pattern text is restricted to ASCII (D-17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alphabet {
    /// Pattern text must be 7-bit ASCII; bytes > 127 are rejected at parse.
    Ascii,
    /// Pattern text is full UTF-8.
    Utf8,
}

impl Dialect {
    /// The canonical stable ASCII id (D-16).
    pub fn id(self) -> &'static str {
        match self {
            Dialect::Posix => "posix",
            Dialect::Win => "win",
        }
    }

    /// The current concrete semver `(major, minor, patch)` version of this dialect
    /// (D-20).
    pub fn version(self) -> (u16, u16, u16) {
        match self {
            Dialect::Posix | Dialect::Win => (1, 0, 0),
        }
    }

    /// The pattern-text alphabet (D-17).
    pub fn alphabet(self) -> Alphabet {
        match self {
            Dialect::Posix | Dialect::Win => Alphabet::Utf8,
        }
    }

    /// The default case-sensitivity when the caller does not override it (D-23).
    pub fn default_case(self) -> CaseSensitivity {
        match self {
            Dialect::Posix => CaseSensitivity::Sensitive,
            Dialect::Win => CaseSensitivity::Insensitive,
        }
    }

    /// Whether this dialect is usable on the current platform (D-47). The gate is a
    /// policy applied by the engine/builder, not by the pure parser/matcher.
    pub fn is_supported(self) -> bool {
        match self {
            Dialect::Posix => true,
            Dialect::Win => cfg!(windows),
        }
    }

    /// Resolve a dialect id (optionally with an `@version` suffix) to a concrete
    /// dialect (D-20). Partial-version binding: `posix`, `posix@1`, `posix@1.0`, and
    /// `posix@1.0.0` all bind to the highest matching concrete version; `posix@2` and
    /// an overlong `posix@1.0.0.0` do not.
    pub fn resolve(spec: &str) -> Option<Dialect> {
        let (id, ver) = match spec.split_once('@') {
            Some((id, ver)) => (id, Some(ver)),
            None => (spec, None),
        };
        let dialect = match id {
            "posix" => Dialect::Posix,
            "win" => Dialect::Win,
            _ => return None,
        };
        match ver {
            Some(v) if !version_matches(v, dialect.version()) => None,
            _ => Some(dialect),
        }
    }
}

/// True if `spec` (a dot-separated version prefix of up to three components) binds to
/// `concrete` (D-20). A prefix longer than a full `major.minor.patch` triplet is not a
/// real version and never binds.
fn version_matches(spec: &str, concrete: (u16, u16, u16)) -> bool {
    let mut parts = Vec::new();
    for p in spec.split('.') {
        match p.parse::<u16>() {
            Ok(n) => parts.push(n),
            Err(_) => return false,
        }
    }
    let (major, minor, patch) = concrete;
    match parts.as_slice() {
        [a] => *a == major,
        [a, b] => *a == major && *b == minor,
        [a, b, c] => *a == major && *b == minor && *c == patch,
        // Empty (`id@`) or an overlong 4+-component spec is not a real version.
        _ => false,
    }
}
