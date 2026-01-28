// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn add_and_remove_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "add_and_remove"],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "urn:kpar:test"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Adding usage: `urn:kpar:test`"));

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
}
"#
    );

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:test"], None)?;

    out.assert().success().stderr(predicate::str::contains(
        r#"Removing `urn:kpar:test` from usages
     Removed `urn:kpar:test`"#,
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "version": "1.2.3",
  "usage": []
}
"#
    );

    Ok(())
}

/// Add and remove usages with `--path <path>`
#[test]
fn add_and_remove_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1, out1) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "add_and_remove_path1",
        ],
        None,
    )?;
    let (_temp_dir2, cwd2, out2) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "add_and_remove_path2",
        ],
        None,
    )?;
    let file_url = file_url_from_path(&cwd2);

    out1.assert().success();
    out2.assert().success();

    let out = run_sysand_in(&cwd1, ["add", "--no-lock", "--path", cwd2.as_str()], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(format!(
            "Adding usage: `{file_url}`",
        )));

    let info_json = std::fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add_and_remove_path1",
  "version": "1.2.3",
  "usage": [
    {{
      "resource": "{}"
    }}
  ]
}}
"#,
            file_url
        )
    );

    let out = run_sysand_in(&cwd1, ["remove", "--path", cwd2.as_str()], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(format!(
            r#"Removing `{}` from usages
     Removed `{}`"#,
            file_url, file_url
        )));

    let info_json = std::fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_path1",
  "version": "1.2.3",
  "usage": []
}
"#
    );

    Ok(())
}

#[test]
fn add_and_remove_with_lock_preinstall() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir_dep, cwd_dep, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "add_and_remove_with_lock_preinstall_dep",
        ],
        None,
    )?;

    out.assert().success();

    std::fs::write(
        cwd_dep.join("add_and_remove_with_lock_preinstall_dep.sysml"),
        "package AddAndRemoveWithLockLocalDep;",
    )?;

    run_sysand_in(
        &cwd_dep,
        ["include", "add_and_remove_with_lock_preinstall_dep.sysml"],
        None,
    )?
    .assert()
    .success();

    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "add_and_remove_with_lock_preinstall",
        ],
        None,
    )?;

    out.assert().success();

    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:add_and_remove_with_lock_preinstall_dep",
            "--path",
            cwd_dep.as_str(),
        ],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        ["add", "urn:kpar:add_and_remove_with_lock_preinstall_dep"],
        None,
    )?
    .assert()
    .success()
    .stderr(predicate::str::contains(
        "Adding usage: `urn:kpar:add_and_remove_with_lock_preinstall_dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_with_lock_preinstall",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:add_and_remove_with_lock_preinstall_dep"
    }
  ]
}
"#
    );

    run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:add_and_remove_with_lock_preinstall_dep"],
        None,
    )?
    .assert()
    .success();

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_with_lock_preinstall",
  "version": "1.2.3",
  "usage": []
}
"#
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
        "could not find usage for `urn:kpar:remove_nonexistent`",
    ));

    Ok(())
}
