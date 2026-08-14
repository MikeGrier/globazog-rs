// Copyright (c) 2026 Mike Grier

//! Reversible, non-panicking decoding of native filesystem names into the 32-bit
//! code-point space (D-46): Windows UTF-16 with unpaired-surrogate preservation
//! (WTF-8 style) and Unix bytes with PEP-383 surrogate-escaping. The bytes are
//! shipped verbatim on the wire (D-63); decoding happens only at match time.

use crate::syntax::CodePoint;

#[cfg(test)]
mod tests;

/// Decode a Windows UTF-16 name into code points, preserving unpaired surrogates
/// as their own code-point value. Never panics.
pub fn decode_utf16(units: &[u16]) -> Vec<CodePoint> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let u = units[i];
        // A high surrogate followed by a low surrogate forms a supplementary char.
        if (0xD800..=0xDBFF).contains(&u)
            && let Some(&lo) = units.get(i + 1)
            && (0xDC00..=0xDFFF).contains(&lo)
        {
            let cp = 0x1_0000 + (((u as u32) - 0xD800) << 10) + ((lo as u32) - 0xDC00);
            out.push(cp);
            i += 2;
            continue;
        }
        // BMP scalar, or an unpaired surrogate preserved by its raw value.
        out.push(u as u32);
        i += 1;
    }
    out
}

/// Decode a Unix byte name into code points. Valid UTF-8 decodes normally; each
/// invalid byte (always `>= 0x80`) is mapped to `U+DC00 + byte` (PEP-383
/// surrogateescape). Never panics.
pub fn decode_bytes(bytes: &[u8]) -> Vec<CodePoint> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    loop {
        match core::str::from_utf8(rest) {
            Ok(s) => {
                out.extend(s.chars().map(|c| c as u32));
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // The prefix up to `valid` is guaranteed well-formed UTF-8.
                if let Ok(s) = core::str::from_utf8(&rest[..valid]) {
                    out.extend(s.chars().map(|c| c as u32));
                }
                // `rest[valid]` is the first byte that breaks UTF-8 (>= 0x80).
                out.push(0xDC00 + rest[valid] as u32);
                rest = &rest[valid + 1..];
            }
        }
    }
}

/// Decode a valid UTF-8 pattern string into code points. Pattern text is always
/// well-formed UTF-8 (D-17), so this is lossless and surrogate-free.
pub fn decode_str(s: &str) -> Vec<CodePoint> {
    s.chars().map(|c| c as u32).collect()
}
