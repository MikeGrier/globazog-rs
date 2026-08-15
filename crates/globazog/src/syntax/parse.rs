// Copyright (c) 2026 Mike Grier

//! Dialect front-ends (D-21, D-22): parse pattern text into the shared segment IR
//! (D-18), applying separator/escape rules, brace alternation (D-44), `**`
//! whole-segment (D-24), `.`-strip / `..`-reject (D-26), and anchor detection
//! (D-25, D-33). `posix` uses `\` escapes; `win` uses `/`+`\` separators with
//! brace-doubling escaping (D-45).

use crate::error::Error;
use crate::syntax::dialect::{Alphabet, Dialect};
use crate::syntax::{Pattern, PatternSegment, Token};

#[cfg(test)]
mod tests;

/// How a parsed pattern is rooted (D-33). Drive-vs-relative and UNC handling is
/// finalized by the builder (M6); the parser records what the text expressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// No leading separator — matched relative to a caller-supplied root.
    Relative,
    /// A leading separator — rooted at the filesystem/current-drive root.
    Root,
    /// `win`: a leading `X:` drive designator.
    Drive(char),
    /// `win`: a leading `\\` — the first two segments are server and share (D-25).
    Unc,
}

/// A parsed pattern: its anchor plus the dialect-independent segment IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPattern {
    /// How the pattern is rooted.
    pub anchor: Anchor,
    /// The compiled segment sequence.
    pub pattern: Pattern,
}

/// Parse `input` in `dialect` into the segment IR (D-18).
pub fn parse(input: &str, dialect: Dialect) -> Result<ParsedPattern, Error> {
    if matches!(dialect.alphabet(), Alphabet::Ascii)
        && let Some(c) = input.chars().find(|&c| c as u32 > 0x7F)
    {
        return Err(Error::Pattern(format!(
            "non-ASCII character {c:?} in ASCII dialect `{}`",
            dialect.id()
        )));
    }

    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    let anchor = detect_anchor(&chars, &mut i, dialect);

    // Split the remainder into raw segments on unescaped separators (collapsing
    // consecutive separators per D-25).
    let mut raw: Vec<Vec<char>> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if dialect == Dialect::Posix && c == '\\' {
            let n = *chars
                .get(i + 1)
                .ok_or_else(|| Error::Pattern("dangling `\\` escape".into()))?;
            cur.push('\\');
            cur.push(n);
            i += 2;
            continue;
        }
        if is_sep(c, dialect) {
            raw.push(std::mem::take(&mut cur));
            i += 1;
            while i < chars.len() && is_sep(chars[i], dialect) {
                i += 1;
            }
            continue;
        }
        cur.push(c);
        i += 1;
    }
    raw.push(cur);

    let mut segments = Vec::new();
    for seg in raw {
        if seg.is_empty() {
            continue;
        }
        if let Some(ps) = classify_segment(&seg, dialect)? {
            segments.push(ps);
        }
    }

    Ok(ParsedPattern {
        anchor,
        pattern: Pattern { segments },
    })
}

fn is_sep(c: char, dialect: Dialect) -> bool {
    match dialect {
        Dialect::Posix => c == '/',
        Dialect::Win => c == '/' || c == '\\',
    }
}

fn detect_anchor(chars: &[char], i: &mut usize, dialect: Dialect) -> Anchor {
    let mut n = 0;
    while *i < chars.len() && is_sep(chars[*i], dialect) {
        n += 1;
        *i += 1;
    }
    match dialect {
        Dialect::Posix => {
            if n > 0 {
                Anchor::Root
            } else {
                Anchor::Relative
            }
        }
        Dialect::Win => {
            if n >= 2 {
                Anchor::Unc
            } else if n == 1 {
                Anchor::Root
            } else if chars.first().is_some_and(char::is_ascii_alphabetic)
                && chars.get(1) == Some(&':')
            {
                let letter = chars[0].to_ascii_uppercase();
                *i = 2;
                while *i < chars.len() && is_sep(chars[*i], dialect) {
                    *i += 1;
                }
                Anchor::Drive(letter)
            } else {
                Anchor::Relative
            }
        }
    }
}

/// Turn one raw segment into a `PatternSegment`, or `None` if it is a `.` to strip.
fn classify_segment(seg: &[char], dialect: Dialect) -> Result<Option<PatternSegment>, Error> {
    let tokens = tokenize(seg, dialect)?;

    // `**` is legal only as a whole segment (D-24).
    if tokens.len() == 2 && tokens[0] == Token::Star && tokens[1] == Token::Star {
        return Ok(Some(PatternSegment::DoubleStar));
    }
    if tokens
        .windows(2)
        .any(|w| w[0] == Token::Star && w[1] == Token::Star)
    {
        return Err(Error::Pattern(
            "`**` is only allowed as a whole path segment".into(),
        ));
    }

    // `.` strip / `..` reject (D-26).
    let dot = Token::Literal('.' as u32);
    if tokens.len() == 1 && tokens[0] == dot {
        return Ok(None);
    }
    if tokens.len() == 2 && tokens[0] == dot && tokens[1] == dot {
        return Err(Error::Pattern("`..` is not allowed in a pattern".into()));
    }

    Ok(Some(PatternSegment::Match(tokens)))
}

fn tokenize(seg: &[char], dialect: Dialect) -> Result<Vec<Token>, Error> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < seg.len() {
        let c = seg[i];
        if dialect == Dialect::Posix && c == '\\' {
            let n = *seg
                .get(i + 1)
                .ok_or_else(|| Error::Pattern("dangling `\\` escape".into()))?;
            toks.push(Token::Literal(n as u32));
            i += 2;
            continue;
        }
        match c {
            '*' => {
                toks.push(Token::Star);
                i += 1;
            }
            '?' => {
                toks.push(Token::Any);
                i += 1;
            }
            '{' if dialect == Dialect::Win && seg.get(i + 1) == Some(&'{') => {
                toks.push(Token::Literal('{' as u32));
                i += 2;
            }
            '{' => {
                let (alt, next) = parse_brace(seg, i + 1, dialect)?;
                toks.push(alt);
                i = next;
            }
            '}' if dialect == Dialect::Win && seg.get(i + 1) == Some(&'}') => {
                toks.push(Token::Literal('}' as u32));
                i += 2;
            }
            '}' => {
                toks.push(Token::Literal('}' as u32));
                i += 1;
            }
            _ => {
                toks.push(Token::Literal(c as u32));
                i += 1;
            }
        }
    }
    Ok(toks)
}

/// Parse a brace alternation body starting just after `{`; returns the `Alt` token
/// and the index just past the closing `}`. Nested braces are unsupported (D-44).
///
/// Limitation (D-45): `win`-dialect brace-doubling (`{{`/`}}`) is **not** honored
/// inside an alternation arm — `{` is always the (rejected) start of a nested group
/// and the first `}` always closes the alternation, so a literal brace cannot appear
/// inside a brace group. Honoring it here is ambiguous with arm termination (`{a,b}}`
/// could not close), so it is deliberately excluded.
fn parse_brace(seg: &[char], mut i: usize, dialect: Dialect) -> Result<(Token, usize), Error> {
    let mut arms: Vec<Vec<Token>> = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    while i < seg.len() {
        let c = seg[i];
        if dialect == Dialect::Posix && c == '\\' {
            let n = *seg
                .get(i + 1)
                .ok_or_else(|| Error::Pattern("dangling `\\` escape".into()))?;
            cur.push(Token::Literal(n as u32));
            i += 2;
            continue;
        }
        match c {
            '}' => {
                arms.push(cur);
                return Ok((Token::Alt(arms), i + 1));
            }
            ',' => {
                arms.push(std::mem::take(&mut cur));
                i += 1;
            }
            '{' => {
                return Err(Error::Pattern(
                    "nested `{}` alternation is unsupported".into(),
                ));
            }
            '*' => {
                cur.push(Token::Star);
                i += 1;
            }
            '?' => {
                cur.push(Token::Any);
                i += 1;
            }
            _ => {
                cur.push(Token::Literal(c as u32));
                i += 1;
            }
        }
    }
    Err(Error::Pattern("unterminated `{` alternation".into()))
}
