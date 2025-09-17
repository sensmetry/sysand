// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand new` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, falling back
/// on directory name as name.
#[test]
fn new_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(&vec!["new", "--version", "1.2.3", "new_basic"], None)?;

    let proj_dir_path = cwd.join("new_basic");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{6}|\d{9})Z"\n}"#,
    )?;

    assert_eq!(
        info,
        "{\n  \"name\": \"new_basic\",\n  \"version\": \"1.2.3\",\n  \"usage\": []\n}"
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand new` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, using explicitly
/// specified name as project name.
#[test]
fn new_explicit_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        &vec![
            "new",
            "--version",
            "1.2.3",
            "--name",
            "other_than_new_explicit_name",
            "new_explicit_name",
        ],
        None,
    )?;

    let proj_dir_path = cwd.join("new_explicit_name");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.(\d{6}|\d{9})Z"\n}"#,
    )?;

    assert_eq!(
        info,
        "{\n  \"name\": \"other_than_new_explicit_name\",\n  \"version\": \"1.2.3\",\n  \"usage\": []\n}"
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand new` should fail (loudly) in case there is already
/// a project present (in the specified directory). Such an existing
/// project should remain unaffected by the second `sysand new` execution.
#[test]
fn new_fail_on_double_new() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) = run_sysand(
        &vec!["new", "--version", "1.2.3", "new_fail_on_double_new"],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    let proj_dir_path = cwd.join("new_fail_on_double_new");

    assert!(proj_dir_path.exists());

    let original_info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        &vec!["new", "--version", "1.2.3", "new_fail_on_double_new"],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("'.project.json' already exists"));

    assert_eq!(
        original_info,
        std::fs::read_to_string(proj_dir_path.join(".project.json"))?
    );
    assert_eq!(
        original_meta,
        std::fs::read_to_string(proj_dir_path.join(".meta.json"))?
    );

    Ok(())
}
