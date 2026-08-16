// Copyright (c) 2026 Mike Grier

use super::{decode_bytes, decode_str, decode_utf16};

#[test]
fn ascii_utf16() {
    assert_eq!(decode_utf16(&[0x41, 0x42, 0x43]), vec![0x41, 0x42, 0x43]);
}

#[test]
fn empty_inputs() {
    assert_eq!(decode_utf16(&[]), Vec::<u32>::new());
    assert_eq!(decode_bytes(&[]), Vec::<u32>::new());
    assert_eq!(decode_str(""), Vec::<u32>::new());
}

#[test]
fn utf16_surrogate_pair() {
    // U+1F600 = D83D DE00
    assert_eq!(decode_utf16(&[0xD83D, 0xDE00]), vec![0x1F600]);
}

#[test]
fn utf16_unpaired_high_surrogate_preserved() {
    assert_eq!(decode_utf16(&[0xD83D, 0x0041]), vec![0xD83D, 0x0041]);
}

#[test]
fn utf16_unpaired_high_at_end() {
    assert_eq!(decode_utf16(&[0x0041, 0xD83D]), vec![0x0041, 0xD83D]);
}

#[test]
fn utf16_lone_low_surrogate_preserved() {
    assert_eq!(decode_utf16(&[0xDE00]), vec![0xDE00]);
}

#[test]
fn utf16_bmp_nonascii() {
    // U+00E9 é
    assert_eq!(decode_utf16(&[0x00E9]), vec![0x00E9]);
}

#[test]
fn bytes_ascii() {
    assert_eq!(decode_bytes(b"abc"), vec![0x61, 0x62, 0x63]);
}

#[test]
fn bytes_valid_utf8_multibyte() {
    // é = C3 A9 -> U+00E9
    assert_eq!(decode_bytes(&[0xC3, 0xA9]), vec![0x00E9]);
}

#[test]
fn bytes_invalid_escaped() {
    // 0xFF is invalid -> U+DCFF
    assert_eq!(decode_bytes(&[0xFF]), vec![0xDCFF]);
}

#[test]
fn bytes_mixed_valid_invalid() {
    // 'a', 0x80 (invalid), 'b'
    assert_eq!(decode_bytes(&[0x61, 0x80, 0x62]), vec![0x61, 0xDC80, 0x62]);
}

#[test]
fn bytes_truncated_multibyte_escaped() {
    // 0xC3 alone is an incomplete sequence -> escaped
    assert_eq!(decode_bytes(&[0xC3]), vec![0xDCC3]);
}

#[test]
fn str_nonascii() {
    assert_eq!(decode_str("aé"), vec![0x61, 0x00E9]);
}
