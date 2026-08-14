// Copyright (c) 2026 Mike Grier

//! Glob syntax: the closed set of named dialects (D-15–D-20), each a front-end that
//! lowers a UTF-8 pattern into the shared segment-structured IR (D-18); the matcher
//! over 32-bit code points (D-46) with `*` / `**` / `?` / n-ary brace alternation
//! (D-24, D-44, D-67); anchor extraction and the pattern-set model (D-36–D-39).
