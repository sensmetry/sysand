// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::io::Write as _;

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// Write a KPAR containing `.project.json`/`.meta.json` at the archive root,
/// as required by the `KparPath` usage type.
fn write_dep_kpar(
    kpar_path: &camino::Utf8Path,
    publisher: &str,
    name: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(kpar_path)?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);

    zip.start_file(".project.json", options)?;
    zip.write_all(
        format!(r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}"}}"#)
            .as_bytes(),
    )?;
    zip.start_file(".meta.json", options)?;
    zip.write_all(br#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)?;

    zip.finish()?;
    Ok(())
}

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

#[test]
fn exp_add_and_remove_kpar_path_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_and_remove_kpar_path", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "b", "my-dep", "1.0.0")?;

    let out = run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `b/my-dep` in `dep.kpar`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove_kpar_path",
  "publisher": "a",
  "version": "1.2.3",
  "usage": [
    {
      "kpar_path": "dep.kpar",
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
        .stderr(predicate::str::contains(
            "Removed `b/my-dep` (path `dep.kpar`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove_kpar_path",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_kpar_path_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_no_publisher", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    {
        let file = std::fs::File::create(&dep_kpar)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o644);
        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"no-publisher-dep","version":"1.0.0"}"#)?;
        zip.finish()?;
    }

    let out = run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("does not have a publisher"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_kpar_path_no_publisher",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn exp_add_kpar_path_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_nonexistent", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    // dep.kpar exists as a valid, but empty, archive: no `.project.json`
    {
        let file = std::fs::File::create(&dep_kpar)?;
        zip::ZipWriter::new(file).finish()?;
    }

    let out = run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));

    Ok(())
}

#[test]
fn exp_add_kpar_path_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_already_present", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "b", "my-dep", "1.0.0")?;

    run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("is already present"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_kpar_path_already_present",
  "publisher": "a",
  "version": "1.2.3",
  "usage": [
    {
      "kpar_path": "dep.kpar",
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
fn exp_remove_kpar_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_kpar_path", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "b", "my-dep", "1.0.0")?;

    run_sysand_in(
        &cwd,
        [
            "experimental",
            "add",
            "--no-lock",
            "--kpar-path",
            "dep.kpar",
        ],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(&cwd, ["experimental", "remove", "b", "my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Removing `b/my-dep` from usages"))
        .stderr(predicate::str::contains(
            "Removed `b/my-dep` (path `dep.kpar`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_remove_kpar_path",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}
