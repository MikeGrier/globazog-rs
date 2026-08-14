// Copyright (c) 2026 Mike Grier

//! Declarative predicates (D-56, D-66): an AND-only vocabulary of signed / set-
//! membership `Leaf` conditions over entry metadata, a flat-conjunction evaluator
//! (D-67), and the lazy-`statx` fetch-mask (D-62). Emit predicates are per-pattern
//! and descend predicates are per-query; their composition with the glob set (D-66)
//! is performed by the engine (M6/M7). Name conditions reuse the single-segment
//! matcher (M2), so they honor the same case rules.

use crate::syntax::matcher::match_segment;
use crate::syntax::{CaseSensitivity, CodePoint, Segment, Token};
use bitflags::bitflags;

#[cfg(test)]
mod tests;

/// The coarse kind of a directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryType {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// Anything else (device, socket, fifo, …).
    Other,
}

/// Which timestamp a [`Leaf::Time`] condition compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeField {
    /// Birth / creation time.
    Btime,
    /// Last-modification time.
    Mtime,
    /// Last-access time.
    Atime,
    /// Metadata-change time (Windows change time / Unix ctime).
    Ctime,
}

/// A comparison operator for numeric [`Leaf`] conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `>=`
    Ge,
    /// `>`
    Gt,
}

impl Cmp {
    fn apply<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Ge => a >= b,
            Cmp::Gt => a > b,
        }
    }
}

bitflags! {
    /// Stat-tier metadata fields a predicate references, driving the engine's lazy
    /// fetch (D-62). Name and depth are always available and are not tracked here.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct MetaMask: u16 {
        /// File size.
        const SIZE    = 1 << 0;
        /// Last-modification time.
        const MTIME   = 1 << 1;
        /// Last-access time.
        const ATIME   = 1 << 2;
        /// Metadata-change time.
        const CTIME   = 1 << 3;
        /// Birth / creation time.
        const BTIME   = 1 << 4;
        /// Entry type.
        const TYPE    = 1 << 5;
        /// Reparse-point status and tag.
        const REPARSE = 1 << 6;
        /// Attribute bitmask.
        const ATTRS   = 1 << 7;
    }
}

/// The metadata a predicate evaluates against. `name` and `depth` are always
/// present; the remaining fields are populated only when a leaf (or the requested
/// result shape) references them (D-62).
#[derive(Clone, Copy, Debug)]
pub struct EntryMeta<'a> {
    /// The entry's own name in code-point space (D-46).
    pub name: &'a [CodePoint],
    /// Depth from the traversal root (0 = a root's direct child).
    pub depth: u32,
    /// The entry kind.
    pub entry_type: EntryType,
    /// Whether the entry is a reparse point (D-13).
    pub is_reparse: bool,
    /// The reparse tag (0 when not a reparse point).
    pub reparse_tag: u32,
    /// The attribute bitmask (Windows `FILE_ATTRIBUTE_*`; sparse on Linux).
    pub attributes: u32,
    /// File size in bytes.
    pub size: u64,
    /// Birth / creation time.
    pub btime: i64,
    /// Last-modification time.
    pub mtime: i64,
    /// Last-access time.
    pub atime: i64,
    /// Metadata-change time.
    pub ctime: i64,
}

impl EntryMeta<'_> {
    fn time(&self, field: TimeField) -> i64 {
        match field {
            TimeField::Btime => self.btime,
            TimeField::Mtime => self.mtime,
            TimeField::Atime => self.atime,
            TimeField::Ctime => self.ctime,
        }
    }
}

/// One signed condition over an entry's metadata (D-56). A conjunction of these is
/// evaluated with [`eval_all`].
#[derive(Clone, Debug)]
pub enum Leaf {
    /// The name matches a single-segment glob; `negate` inverts the result.
    Name {
        /// The single-segment matcher (literals / `*` / `?` / alternation).
        seg: Segment,
        /// Case rule for the comparison.
        case: CaseSensitivity,
        /// Invert the match.
        negate: bool,
    },
    /// The name matches any glob in the set; `negate` inverts (i.e. `∉`).
    NameInSet {
        /// The alternatives to match against.
        segs: Vec<Segment>,
        /// Case rule for the comparison.
        case: CaseSensitivity,
        /// Invert to set non-membership.
        negate: bool,
    },
    /// The entry is (or, negated, is not) the given type.
    IsType {
        /// The type to test.
        ty: EntryType,
        /// Invert the test.
        negate: bool,
    },
    /// The entry is (or is not) a reparse point (D-13, D-52).
    IsReparse {
        /// Invert the test.
        negate: bool,
    },
    /// The reparse tag equals (or, negated, differs from) `tag`.
    ReparseTag {
        /// The tag to compare.
        tag: u32,
        /// Invert the test.
        negate: bool,
    },
    /// Every bit in `mask` is set in the attributes.
    AttrsAllSet(u32),
    /// Every bit in `mask` is clear in the attributes.
    AttrsAllClear(u32),
    /// The size compares to `value` via `op`.
    Size {
        /// The comparison operator.
        op: Cmp,
        /// The size to compare against, in bytes.
        value: u64,
    },
    /// The `field` timestamp compares to `value` via `op`.
    Time {
        /// Which timestamp to compare.
        field: TimeField,
        /// The comparison operator.
        op: Cmp,
        /// The timestamp to compare against.
        value: i64,
    },
    /// The depth compares to `value` via `op`.
    Depth {
        /// The comparison operator.
        op: Cmp,
        /// The depth to compare against.
        value: u32,
    },
}

fn literals(s: &str) -> Segment {
    s.chars().map(|c| Token::Literal(c as u32)).collect()
}

impl Leaf {
    /// The name equals `name` exactly.
    pub fn name_exact(name: &str, case: CaseSensitivity) -> Leaf {
        Leaf::Name {
            seg: literals(name),
            case,
            negate: false,
        }
    }

    /// The name matches a compiled single-segment glob.
    pub fn name_glob(seg: Segment, case: CaseSensitivity) -> Leaf {
        Leaf::Name {
            seg,
            case,
            negate: false,
        }
    }

    /// The name contains `needle` as a substring.
    pub fn name_contains(needle: &str, case: CaseSensitivity) -> Leaf {
        let mut seg = vec![Token::Star];
        seg.extend(literals(needle));
        seg.push(Token::Star);
        Leaf::Name {
            seg,
            case,
            negate: false,
        }
    }

    /// The name has the given extension (matches `*.<ext>`).
    pub fn name_extension(ext: &str, case: CaseSensitivity) -> Leaf {
        let mut seg = vec![Token::Star, Token::Literal('.' as u32)];
        seg.extend(literals(ext));
        Leaf::Name {
            seg,
            case,
            negate: false,
        }
    }

    /// The name equals any of `names`.
    pub fn name_in_set(names: &[&str], case: CaseSensitivity) -> Leaf {
        Leaf::NameInSet {
            segs: names.iter().map(|n| literals(n)).collect(),
            case,
            negate: false,
        }
    }
}

/// Evaluate a single leaf against an entry's metadata.
pub fn eval_leaf(leaf: &Leaf, meta: &EntryMeta) -> bool {
    match leaf {
        Leaf::Name { seg, case, negate } => match_segment(seg, meta.name, *case) ^ negate,
        Leaf::NameInSet { segs, case, negate } => {
            segs.iter().any(|s| match_segment(s, meta.name, *case)) ^ negate
        }
        Leaf::IsType { ty, negate } => (meta.entry_type == *ty) ^ negate,
        Leaf::IsReparse { negate } => meta.is_reparse ^ negate,
        Leaf::ReparseTag { tag, negate } => (meta.reparse_tag == *tag) ^ negate,
        Leaf::AttrsAllSet(mask) => meta.attributes & mask == *mask,
        Leaf::AttrsAllClear(mask) => meta.attributes & mask == 0,
        Leaf::Size { op, value } => op.apply(meta.size, *value),
        Leaf::Time { field, op, value } => op.apply(meta.time(*field), *value),
        Leaf::Depth { op, value } => op.apply(meta.depth, *value),
    }
}

/// Evaluate a flat conjunction (D-67): true iff every leaf holds. An empty list is
/// vacuously true.
pub fn eval_all(leaves: &[Leaf], meta: &EntryMeta) -> bool {
    leaves.iter().all(|leaf| eval_leaf(leaf, meta))
}

fn leaf_fields(leaf: &Leaf) -> MetaMask {
    match leaf {
        Leaf::Name { .. } | Leaf::NameInSet { .. } | Leaf::Depth { .. } => MetaMask::empty(),
        Leaf::IsType { .. } => MetaMask::TYPE,
        Leaf::IsReparse { .. } | Leaf::ReparseTag { .. } => MetaMask::REPARSE,
        Leaf::AttrsAllSet(_) | Leaf::AttrsAllClear(_) => MetaMask::ATTRS,
        Leaf::Size { .. } => MetaMask::SIZE,
        Leaf::Time { field, .. } => match field {
            TimeField::Btime => MetaMask::BTIME,
            TimeField::Mtime => MetaMask::MTIME,
            TimeField::Atime => MetaMask::ATIME,
            TimeField::Ctime => MetaMask::CTIME,
        },
    }
}

/// The union of stat-tier fields referenced by a conjunction (D-62). The engine
/// unions this across every emit list, the descend list, and the requested result
/// shape to decide what to fetch per entry.
pub fn required_fields(leaves: &[Leaf]) -> MetaMask {
    leaves
        .iter()
        .fold(MetaMask::empty(), |acc, leaf| acc | leaf_fields(leaf))
}
