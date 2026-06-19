// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, falling back
/// on directory name as name.
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--publisher",
            "a",
            "init_basic",
        ],
        None,
    )?;

    let proj_dir_path = cwd.join("init_basic");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "init_basic",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init`, when not given a version, should default to `0.0.1`.
#[test]
fn init_default_version() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project(Some("init_default_version"), "a", None, None, None)?;

    let proj_dir_path = cwd.join("init_default_version");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;

    assert_eq!(
        info,
        r#"{
  "name": "init_default_version",
  "publisher": "a",
  "version": "0.0.1"
}
"#
    );

    Ok(())
}

/// `sysand init`, when not given a directory, should create a
/// project in cwd.
#[test]
fn init_basic_cwd() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("b", "init_basic_cwd", "1.2.3")?;

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "init_basic_cwd",
  "publisher": "b",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, using explicitly
/// specified name as project name.
#[test]
fn init_explicit_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--publisher",
            "c",
            "--name",
            "other_than_init_explicit_name",
            "init_explicit_name",
        ],
        None,
    )?;

    let proj_dir_path = cwd.join("init_explicit_name");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "other_than_init_explicit_name",
  "publisher": "c",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the specified directory). Such an existing
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "a",
            "--version",
            "1.2.3",
            "init_fail_on_double_init",
        ],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    let proj_dir_path = cwd.join("init_fail_on_double_init");

    assert!(proj_dir_path.exists());

    let original_info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "a",
            "--version",
            "1.2.3",
            "init_fail_on_double_init",
        ],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("`.project.json` already exists"));

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

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the current working directory). The current
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init_cwd() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "init_fail_on_double_init_cwd", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    let original_info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "a",
            "--name",
            "init_fail_on_double_init_cwd_again",
            "--version",
            "3.2.1",
        ],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("`.project.json` already exists"));

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
