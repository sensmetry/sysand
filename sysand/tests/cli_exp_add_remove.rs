// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn exp_add_and_remove_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "exp_add_and_remove"],
        None,
    )?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    run_sysand_in(
        &dep_dir,
        ["init", "--version", "1.0.0", "--name", "my-dep"],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `untitled/my-dep` from `dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "untitled",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "untitled",
      "name": "my-dep"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["experimental", "remove", "untitled", "my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `untitled/my-dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `untitled/my-dep` (path `dep`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "untitled",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "exp_add_no_publisher",
        ],
        None,
    )?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    std::fs::write(
        dep_dir.join(".project.json"),
        r#"{
  "name": "no-publisher-dep",
  "version": "1.0.0"
}
"#,
    )?;

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("does not have a publisher"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_no_publisher",
  "publisher": "untitled",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "exp_add_nonexistent",
        ],
        None,
    )?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    // dep exists as a directory but has no .project.json

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));

    Ok(())
}

#[test]
fn exp_add_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "exp_add_already_present",
        ],
        None,
    )?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    run_sysand_in(
        &dep_dir,
        ["init", "--version", "1.0.0", "--name", "my-dep"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("is already present"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_already_present",
  "publisher": "untitled",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "untitled",
      "name": "my-dep"
    }
  ]
}
"#
    );

    Ok(())
}

#[test]
fn exp_remove() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "exp_remove"],
        None,
    )?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    run_sysand_in(
        &dep_dir,
        ["init", "--version", "1.0.0", "--name", "my-dep"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "remove", "untitled", "my-dep"],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `untitled/my-dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `untitled/my-dep` (path `dep`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_remove",
  "publisher": "untitled",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "exp_remove_nonexistent",
        ],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "remove", "untitled", "nonexistent"],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "could not find usage for `untitled/nonexistent`",
    ));

    Ok(())
}
