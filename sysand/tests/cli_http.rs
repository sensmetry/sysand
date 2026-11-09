// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    net::TcpListener,
    time::{Duration, Instant},
};

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use predicates::prelude::{predicate::str::contains, *};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// An index that accepts connections and never answers fails the command
/// after the read timeout, once per attempt, instead of hanging it.
#[test]
fn stalled_index_times_out() -> Result<(), Box<dyn std::error::Error>> {
    // A legacy project: a `model.toml` one cannot declare a resource usage
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "echo", "stalled-index-times-out", "1.2.3", None)?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let index = format!("http://{}", listener.local_addr()?);
    // Kept open, so the client waits for a response
    std::thread::spawn(move || {
        let _held: Vec<_> = listener.incoming().collect();
    });

    let started = Instant::now();
    let out = run_sysand_in_with(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:stalled_index_times_out",
            "--default-index",
            &index,
        ],
        None,
        &IndexMap::from([("SYSAND_HTTP_TIMEOUT", "1"), ("SYSAND_HTTP_RETRIES", "1")]),
    )?;

    out.assert()
        .failure()
        .stderr(contains("timed out), retrying in").and(contains("(1/1)")));
    assert!(started.elapsed() < Duration::from_secs(30));

    Ok(())
}

#[test]
fn malformed_http_variable_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("echo", "malformed-http-variable-is-an-error", "1.2.3")?;
    out.assert().success();

    let out = run_sysand_in_with(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:malformed_http_variable_is_an_error",
        ],
        None,
        &IndexMap::from([("SYSAND_HTTP_CONNECT_TIMEOUT", "soon")]),
    )?;

    out.assert().failure().stderr(contains(
        "invalid value `soon` for SYSAND_HTTP_CONNECT_TIMEOUT",
    ));

    Ok(())
}
