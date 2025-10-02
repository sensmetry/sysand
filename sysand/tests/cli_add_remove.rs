// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn add_and_remove() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "add_and_remove"],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "urn:kpar:test"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Adding usage: urn:kpar:test"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}"#
    );

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:test"], None)?;

    out.assert().success().stderr(predicate::str::contains(
        r#"Removing urn:kpar:test from usages
             urn:kpar:test"#,
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "version": "1.2.3",
  "usage": []
}"#
    );

    Ok(())
}

#[test]
fn remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "remove_nonexistent"],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:remove_nonexistent"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "could not find usage for urn:kpar:remove_nonexistent",
    ));

    Ok(())
}
