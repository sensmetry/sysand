// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn exp_add_and_remove_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_and_remove", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(&dep_dir, None, "b", Some("my-dep"), Some("1.0.0"), None)?
        .assert()
        .success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `b/my-dep` from `dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "a",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "b",
      "name": "my-dep"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["experimental", "remove", "b", "my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Removing `b/my-dep` from usages"))
        .stderr(predicate::str::contains("Removed `b/my-dep` (path `dep`)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_no_publisher", "1.2.3")?;
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
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_nonexistent", "1.2.3")?;
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_already_present", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(&dep_dir, None, "b", Some("my-dep"), Some("1.0.0"), None)?
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
  "publisher": "a",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "b",
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(&dep_dir, None, "b", Some("my-dep"), Some("1.0.0"), None)?
        .assert()
        .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "dep"],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(&cwd, ["experimental", "remove", "b", "my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Removing `b/my-dep` from usages"))
        .stderr(predicate::str::contains("Removed `b/my-dep` (path `dep`)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_remove",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_nonexistent", "1.2.3")?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["experimental", "remove", "a", "nonexistent"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "could not find usage for `a/nonexistent`",
    ));

    Ok(())
}
