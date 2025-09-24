// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the current working directory. (Non-interactive use)
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        run_sysand(["init", "--name", "init_basic", "--version", "1.2.3"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{6}|\d{9})Z"\n}"#,
    )?;

    assert_eq!(
        info,
        "{\n  \"name\": \"init_basic\",\n  \"version\": \"1.2.3\",\n  \"usage\": []\n}"
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the current working directory). The current
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--name",
            "init_fail_on_double_init",
            "--version",
            "1.2.3",
        ],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    let original_info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        [
            "init",
            "--name",
            "init_fail_on_double_init_again",
            "--version",
            "3.2.1",
        ],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("'.project.json' already exists"));

    assert_eq!(
        original_info,
        std::fs::read_to_string(cwd.join(".project.json"))?
    );
    assert_eq!(
        original_meta,
        std::fs::read_to_string(cwd.join(".meta.json"))?
    );

    Ok(())
}
