// Copyright (c) 2026 Mike Grier

//! The public query builder (D-31–D-34): accepts absolute patterns / a CWD base and
//! lowers to the core query-def (roots + relative patterns) consumed by the engine;
//! `submit` compiles the pattern set (D-66).
