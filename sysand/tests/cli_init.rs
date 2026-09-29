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

/// An invalid license is rejected by both `sysand init --license` and
/// `sysand info license --set` with the same message, where the SPDX
/// caret diagram starts on its own line so that it stays aligned.
#[test]
fn invalid_license_error_is_consistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--publisher", "a", "--license", "MIT ANDD foo"],
        None,
    )?;
    let expected = "not a valid SPDX license expression:\nMIT ANDD foo\n    ^^^^ unknown term\n";
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));
    assert!(!cwd.join(".project.json").exists());

    let out = run_sysand_in(&cwd, ["init", "--publisher", "a"], None)?;
    out.assert().success();
    let out = run_sysand_in(&cwd, ["info", "license", "--set", "MIT ANDD foo"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));

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

/// `sysand init` should reject an invalid `--publisher` before creating
/// anything
#[test]
fn init_rejects_invalid_publisher() -> Result<(), Box<dyn std::error::Error>> {
    for (publisher, msg) in [
        ("", "publisher cannot be empty"),
        ("acme/labs", "publisher cannot contain `/`"),
        ("acme:labs", "publisher cannot contain `:`"),
        ("acme\tlabs", "publisher cannot contain control characters"),
    ] {
        let (_temp_dir, cwd, out) =
            run_sysand(["init", "--publisher", publisher, "--name", "n", "p"], None)?;

        out.assert().failure().stderr(
            predicate::str::contains(format!(
                "invalid value '{publisher}' for '--publisher <PUBLISHER>'"
            ))
            .and(predicate::str::contains(msg)),
        );
        assert!(!cwd.join("p").exists(), "publisher: {publisher:?}");
    }

    Ok(())
}

/// `sysand init` should reject an invalid `--name` before creating
/// anything
#[test]
fn init_rejects_invalid_name() -> Result<(), Box<dyn std::error::Error>> {
    for (name, msg) in [
        ("", "name cannot be empty"),
        ("a/b", "name cannot contain `/`"),
        ("a:b", "name cannot contain `:`"),
        ("a\nb", "name cannot contain control characters"),
    ] {
        let (_temp_dir, cwd, out) =
            run_sysand(["init", "--publisher", "a", "--name", name, "p"], None)?;

        out.assert().failure().stderr(
            predicate::str::contains(format!("invalid value '{name}' for '--name <NAME>'"))
                .and(predicate::str::contains(msg)),
        );
        assert!(!cwd.join("p").exists(), "name: {name:?}");
    }

    Ok(())
}

/// `sysand init` should accept a publisher and name containing spaces
#[test]
fn init_accepts_spaces_in_publisher_and_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "Acme Labs",
            "--name",
            "My Project",
            "p",
        ],
        None,
    )?;

    out.assert().success();
    let info = std::fs::read_to_string(cwd.join("p").join(".project.json"))?;
    assert_eq!(
        info,
        r#"{
  "name": "My Project",
  "publisher": "Acme Labs",
  "version": "0.0.1"
}
"#
    );

    Ok(())
}

/// `sysand init` without `--name` should reject a directory name that is
/// not a valid project name, and suggest `--name`. `:` is not allowed in
/// Windows file names, hence Unix only
#[cfg(unix)]
#[test]
fn init_rejects_invalid_directory_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(["init", "--publisher", "a", "bad:dir"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "cannot use the directory name `bad:dir` as the project name: \
         name cannot contain `:`; use `--name` to set it",
    ));
    assert!(!cwd.join("bad:dir").join(".project.json").exists());

    let out = run_sysand_in(
        &cwd,
        ["init", "--publisher", "a", "--name", "good", "bad:dir"],
        None,
    )?;
    out.assert().success();
    assert!(cwd.join("bad:dir").join(".project.json").exists());

    Ok(())
}
