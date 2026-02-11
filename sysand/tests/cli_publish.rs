// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn test_publish_missing_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.0.0", "--name", "test_publish"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["publish"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("kpar file not found"))
        .stderr(predicate::str::contains("sysand build"));

    Ok(())
}

#[test]
fn test_publish_explicit_missing_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.0.0", "--name", "test_publish"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["publish", "nonexistent.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("kpar file not found"));

    Ok(())
}

#[test]
fn test_publish_network_error() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.0.0", "--name", "test_publish"],
        None,
    )?;
    out.assert().success();

    // Include a file and build
    std::fs::write(cwd.join("test.sysml"), "package P;\n")?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    // Try to publish to a non-existent server
    let out = run_sysand_in(&cwd, ["publish", "--index", "http://localhost:1"], None)?;

    out.assert().failure();

    Ok(())
}
