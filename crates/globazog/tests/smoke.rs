// Copyright (c) 2026 Mike Grier

//! Smoke test: the crate links and its public error type constructs.

use globazog::error::Error;

#[test]
fn crate_links_and_error_displays() {
    let err = Error::Pattern("example".to_owned());
    assert!(err.to_string().contains("example"));
}
