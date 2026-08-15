// Copyright (c) 2026 Mike Grier

//! The public query builder (D-31–D-34): accepts absolute patterns / a CWD base and
//! lowers to the core query-def (roots + relative patterns) consumed by the engine;
//! `submit` compiles the pattern set (D-66).
//!
//! The **engine core does zero path resolution** (D-31): it evaluates each
//! *relative* pattern against a path relative to its physical *root* seed. This
//! builder is the utility layer (D-34) that turns caller-facing absolute patterns
//! and a CWD base into that `roots × relative-patterns` core form, peeling the
//! leading literal prefix of a self-rooting pattern into its root (D-37). It reads
//! no process-global state (D-32); the caller supplies roots and any base.

use crate::error::Error;
use crate::predicate::{Leaf, MetaMask, required_fields};
use crate::ring::{CompletionRing, Decision, DecisionToken, SqOp, SubmissionQueue};
use crate::syntax::anchor::literal_of;
use crate::syntax::dialect::Dialect;
use crate::syntax::parse::{Anchor, parse};
use crate::syntax::set::CompiledPattern;
use crate::syntax::{CaseSensitivity, CodePoint, Pattern, PatternSegment};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A physical traversal seed (D-31): a concrete directory the engine opens and
/// walks. Never a pattern — all globbing is relative to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    /// The seed directory path.
    pub path: PathBuf,
}

impl Root {
    /// A root at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// One compiled pattern plus its per-pattern emit filter (D-66). The dialect and
/// case rule live inside [`CompiledPattern`].
#[derive(Clone, Debug)]
pub struct PatternEntry {
    /// The compiled, root-relative glob.
    pub glob: CompiledPattern,
    /// The per-pattern emit conjunction (empty = pass-all, D-66).
    pub emit: Vec<Leaf>,
}

/// Per-query execution options (D-66).
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// In-flight scan permits (D-8).
    pub permits: usize,
    /// Completion-ring capacity (D-68).
    pub ring_capacity: usize,
    /// Whether to run reparse-cycle detection (D-51).
    pub cycle_detection: bool,
    /// The query-level case default applied when a pattern gives no override (D-23).
    pub default_case: Option<CaseSensitivity>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            permits: 64,
            ring_capacity: 1024,
            cycle_detection: true,
            default_case: None,
        }
    }
}

/// The core query definition (D-66): roots + relative patterns only. This is the
/// builder's lowered output, consumed by the engine.
#[derive(Clone, Debug)]
pub struct Query {
    /// The physical seed roots.
    pub roots: Vec<Root>,
    /// The compiled patterns with their emit filters.
    pub patterns: Vec<PatternEntry>,
    /// The per-query descend conjunction (D-66).
    pub descend: Vec<Leaf>,
    /// The requested result-shape metadata (D-62).
    pub result_shape: MetaMask,
    /// Execution options.
    pub options: Options,
}

impl Query {
    /// The engine's per-entry fetch mask (D-66): the union of every pattern's emit
    /// fields, the descend fields, and the requested result shape.
    pub fn fetch_mask(&self) -> MetaMask {
        let mut mask = self.result_shape | required_fields(&self.descend);
        for p in &self.patterns {
            mask |= required_fields(&p.emit);
        }
        mask
    }
}

/// A live query: the completion ring the client services (D-60) and the submission
/// queue for cancellation / decision answers (D-61, D-58). The engine runs on
/// background threads; dropping the handle submits a cancel and joins them (the
/// RAII teardown of D-61, performed by the owned `EngineHandle`).
pub struct QueryHandle {
    completions: Arc<CompletionRing>,
    submissions: Arc<SubmissionQueue>,
    // Drops last, cancelling and joining the engine threads (D-61).
    _engine: crate::engine::EngineHandle,
}

impl QueryHandle {
    /// The completion ring to service (D-60).
    pub fn completions(&self) -> &Arc<CompletionRing> {
        &self.completions
    }

    /// The submission queue for control ops (D-66).
    pub fn submissions(&self) -> &Arc<SubmissionQueue> {
        &self.submissions
    }

    /// Submit a cancel (D-61). Acknowledged by a terminal CQ marker that lands after
    /// all items already queued.
    pub fn cancel(&self) {
        self.submissions.submit(SqOp::Cancel);
    }

    /// Answer a `defer-to-client` request (D-58).
    pub fn answer(&self, token: DecisionToken, decision: Decision) {
        self.submissions
            .submit(SqOp::DecisionAnswer { token, decision });
    }
}

/// A pattern awaiting lowering.
#[derive(Clone, Debug)]
struct PendingPattern {
    text: String,
    dialect: Dialect,
    emit: Vec<Leaf>,
    case: Option<CaseSensitivity>,
}

/// The public builder (D-34). Collect roots, a base, and patterns, then
/// [`build`](Self::build) to the core [`Query`] or [`submit`](Self::submit) to a
/// running [`QueryHandle`].
#[derive(Clone, Debug, Default)]
pub struct QueryBuilder {
    roots: Vec<PathBuf>,
    base: Option<PathBuf>,
    patterns: Vec<PendingPattern>,
    descend: Vec<Leaf>,
    result_shape: MetaMask,
    options: Options,
}

impl QueryBuilder {
    /// A new, empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a physical seed root for relative patterns.
    pub fn root(mut self, path: impl Into<PathBuf>) -> Self {
        self.roots.push(path.into());
        self
    }

    /// Set the base directory used to root a self-rooting pattern that needs one
    /// (e.g. a `win` leading-separator, current-drive-relative pattern). The one
    /// honest one-shot global read (CWD) happens at the caller's edge (D-34).
    pub fn base(mut self, path: impl Into<PathBuf>) -> Self {
        self.base = Some(path.into());
        self
    }

    /// Add a pattern with its dialect and emit filter (dialect default case, D-23).
    pub fn pattern(mut self, text: &str, dialect: Dialect, emit: Vec<Leaf>) -> Self {
        self.patterns.push(PendingPattern {
            text: text.to_string(),
            dialect,
            emit,
            case: None,
        });
        self
    }

    /// Add a pattern with an explicit case override (D-23).
    pub fn pattern_cased(
        mut self,
        text: &str,
        dialect: Dialect,
        case: CaseSensitivity,
        emit: Vec<Leaf>,
    ) -> Self {
        self.patterns.push(PendingPattern {
            text: text.to_string(),
            dialect,
            emit,
            case: Some(case),
        });
        self
    }

    /// Set the per-query descend conjunction (D-66).
    pub fn descend(mut self, leaves: Vec<Leaf>) -> Self {
        self.descend = leaves;
        self
    }

    /// Set the requested result-shape metadata (D-62).
    pub fn result_shape(mut self, mask: MetaMask) -> Self {
        self.result_shape = mask;
        self
    }

    /// Set the execution options.
    pub fn options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Lower to the core [`Query`], compiling every pattern (fallible, D-66).
    ///
    /// Relative patterns bind to the explicit roots; each self-rooting pattern
    /// contributes its own derived root. The core model is orthogonal — the engine
    /// applies the pattern set across the root set (D-36) — so mixing absolute and
    /// relative patterns in one query may cross-apply; isolate an absolute pattern
    /// by submitting it in its own query.
    pub fn build(self) -> Result<Query, Error> {
        if self.patterns.is_empty() {
            return Err(Error::Pattern("query has no patterns".into()));
        }

        let mut roots: Vec<Root> = self.roots.iter().cloned().map(Root::new).collect();
        let explicit_roots = roots.len();
        let mut patterns: Vec<PatternEntry> = Vec::new();
        let mut has_relative = false;

        for pend in &self.patterns {
            if !pend.dialect.is_supported() {
                return Err(Error::Pattern(format!(
                    "dialect `{}` is not supported on this platform",
                    pend.dialect.id()
                )));
            }
            let parsed = parse(&pend.text, pend.dialect)?;
            let case = pend
                .case
                .or(self.options.default_case)
                .unwrap_or_else(|| pend.dialect.default_case());

            let (relative, anchor) = match parsed.anchor {
                Anchor::Relative => {
                    has_relative = true;
                    (parsed.pattern, Anchor::Relative)
                }
                other => {
                    let (root, relative) =
                        lower_self_rooting(other, parsed.pattern, &self.base, pend.dialect)?;
                    intern_root(&mut roots, root);
                    (relative, other)
                }
            };

            patterns.push(PatternEntry {
                glob: CompiledPattern {
                    pattern: relative,
                    anchor,
                    dialect: pend.dialect,
                    case,
                },
                emit: pend.emit.clone(),
            });
        }

        if has_relative && explicit_roots == 0 {
            return Err(Error::Pattern(
                "relative pattern needs a root; call `.root(..)`".into(),
            ));
        }
        if roots.is_empty() {
            return Err(Error::Pattern("query has no roots".into()));
        }

        Ok(Query {
            roots,
            patterns,
            descend: self.descend,
            result_shape: self.result_shape,
            options: self.options,
        })
    }

    /// Build and start the engine, returning a [`QueryHandle`] whose completion ring
    /// the client services while the walk runs on background threads (D-3, D-60).
    pub fn submit(self) -> Result<QueryHandle, Error> {
        let ring_capacity = self.options.ring_capacity;
        let query = self.build()?;
        let completions = Arc::new(CompletionRing::with_capacity(ring_capacity));
        let submissions = Arc::new(SubmissionQueue::new());
        let engine =
            crate::engine::spawn(query, Arc::clone(&completions), Arc::clone(&submissions));
        Ok(QueryHandle {
            completions,
            submissions,
            _engine: engine,
        })
    }
}

/// Intern `root` into `roots`, returning its index (dedups equal seeds, D-35).
fn intern_root(roots: &mut Vec<Root>, root: Root) -> usize {
    if let Some(i) = roots.iter().position(|r| *r == root) {
        i
    } else {
        roots.push(root);
        roots.len() - 1
    }
}

/// Lower a self-rooting pattern (D-33/D-34): derive its physical root from the
/// anchor, peel the leading literal segment prefix into that root (D-37), and
/// return the relative remainder.
fn lower_self_rooting(
    anchor: Anchor,
    pattern: Pattern,
    base: &Option<PathBuf>,
    dialect: Dialect,
) -> Result<(Root, Pattern), Error> {
    let mut root = match anchor {
        Anchor::Root => {
            // A leading separator. With a base, root at the base's filesystem root.
            // Without a base: a `win` leading separator is current-drive-relative
            // (process-global state we refuse to read, D-32/D-35), so reject it; a
            // posix `/` is an honest absolute root that needs none.
            match base {
                Some(b) => root_of(b),
                None => {
                    if dialect == Dialect::Win {
                        return Err(Error::Pattern(
                            "a leading-separator `win` pattern is current-drive-relative \
                             and needs a per-drive base; supply one with `.base(...)` \
                             (D-32)"
                                .into(),
                        ));
                    }
                    PathBuf::from(std::path::MAIN_SEPARATOR_STR)
                }
            }
        }
        Anchor::Drive(c) => PathBuf::from(format!("{c}:\\")),
        Anchor::Unc => PathBuf::from(r"\\"),
        Anchor::Relative => unreachable!("relative handled by caller"),
    };

    let segments = pattern.segments;
    let mut rest_start = 0usize;

    if matches!(anchor, Anchor::Unc) {
        // The first two segments are the mandatory server and share (D-25).
        let server = literal_segment(&segments, 0)
            .ok_or_else(|| Error::Pattern("UNC pattern requires a literal server name".into()))?;
        let share = literal_segment(&segments, 1)
            .ok_or_else(|| Error::Pattern("UNC pattern requires a literal share name".into()))?;
        root.push(server);
        root.push(share);
        rest_start = 2;
    }

    // Peel leading literal segments into the root (D-37), but always leave at least
    // one segment in the relative pattern: an all-literal absolute pattern like
    // `/etc/hosts` must root at `/etc` and match `hosts`, not root at the file itself
    // and try to enumerate it as a directory (which would never report the file).
    while rest_start + 1 < segments.len() {
        let Some(lit) = literal_segment(&segments, rest_start) else {
            break;
        };
        root.push(lit);
        rest_start += 1;
    }

    let relative = Pattern {
        segments: segments[rest_start..].to_vec(),
    };
    Ok((Root::new(root), relative))
}

/// The root component of `path` (drive/UNC/`/` prefix), used to resolve a `win`
/// current-drive-relative anchor against a caller base (D-34).
fn root_of(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut root = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => root.push(comp.as_os_str()),
            _ => break,
        }
    }
    if root.as_os_str().is_empty() {
        PathBuf::from(std::path::MAIN_SEPARATOR_STR)
    } else {
        root
    }
}

/// If segment `idx` is a pure-literal `Match`, return it as an OS string.
fn literal_segment(segments: &[PatternSegment], idx: usize) -> Option<String> {
    match segments.get(idx)? {
        PatternSegment::Match(seg) => literal_of(seg).map(|cps| code_points_to_string(&cps)),
        PatternSegment::DoubleStar => None,
    }
}

/// Render a literal code-point run to a `String` (literals are matcher input, so
/// well-formed by construction; any stray unit maps to U+FFFD).
fn code_points_to_string(cps: &[CodePoint]) -> String {
    cps.iter()
        .map(|&c| char::from_u32(c).unwrap_or('\u{FFFD}'))
        .collect()
}
