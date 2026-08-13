// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{fs, io::Write as _};

use assert_cmd::prelude::*;
use predicates::prelude::*;
use sysand_core::env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH};

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

/// `experimental add --no-sync` must update the lockfile but must not touch
/// `.sysand` at all.
#[test]
fn exp_add_no_sync_skips_env_sync() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_no_sync_app", "1.0.0")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "b",
        Some("exp_add_no_sync_dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-sync", "--dir", "dep"],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Adding usage: `b/exp_add_no_sync_dep` from `dep`",
        ))
        .stderr(predicate::str::contains("Syncing").not())
        .stderr(predicate::str::contains("Creating env").not());

    let lockfile =
        fs::read_to_string(cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME))?;
    assert!(
        lockfile.contains("exp_add_no_sync_dep"),
        "lockfile must still be generated by `experimental add --no-sync`: {lockfile}"
    );

    assert!(
        !cwd.join(DEFAULT_ENV_NAME).exists(),
        "`experimental add --no-sync` must not create `.sysand`"
    );

    Ok(())
}

/// `experimental add` must remove a project from `.sysand` once it is no
/// longer present in the freshly regenerated lockfile, while leaving
/// dependencies that are still needed (and the one being added) alone.
#[test]
fn exp_add_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "exp_add_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "exp_add_prune_drop", "1.0.0")?;
    out.assert().success();

    let new_dir = cwd.join("new_dep");
    fs::create_dir_all(&new_dir)?;
    cli_init_project_in(
        &new_dir,
        None,
        "b",
        Some("exp_add_prune_new"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:exp-add-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:exp-add-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();
    run_sysand_in(&cwd, ["sync"], cfg)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("kpar.exp-add-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.exp-add-prune-drop_1.0.0").is_dir());

    // Drop the usage and regenerate the lockfile without touching the env.
    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "urn:kpar:exp-add-prune-drop"],
        cfg,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();

    // `experimental add` triggers a full relock + sync; by default this must
    // prune `exp-add-prune-drop`, which is no longer in the lockfile.
    run_sysand_in(&cwd, ["experimental", "add", "--dir", "new_dep"], cfg)?
        .assert()
        .success();

    assert!(
        env_lib.join("kpar.exp-add-prune-keep_1.0.0").is_dir(),
        "still-needed dependency must not be pruned"
    );
    assert!(
        env_lib.join("b.exp_add_prune_new_1.0.0").is_dir(),
        "newly added dependency must be installed"
    );
    assert!(
        !env_lib.join("kpar.exp-add-prune-drop_1.0.0").exists(),
        "unneeded dependency must be pruned from `.sysand` by default"
    );

    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    assert!(env_toml.contains("exp-add-prune-keep"));
    assert!(!env_toml.contains("exp-add-prune-drop"));

    Ok(())
}

/// `experimental add --no-prune` must leave a dependency that is no longer
/// present in the freshly regenerated lockfile installed in `.sysand`.
#[test]
fn exp_add_no_prune_keeps_unneeded_dependency() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_no_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "exp_add_no_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "exp_add_no_prune_drop", "1.0.0")?;
    out.assert().success();

    let new_dir = cwd.join("new_dep");
    fs::create_dir_all(&new_dir)?;
    cli_init_project_in(
        &new_dir,
        None,
        "b",
        Some("exp_add_no_prune_new"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:exp-add-no-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:exp-add-no-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();
    run_sysand_in(&cwd, ["sync"], cfg)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("kpar.exp-add-no-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.exp-add-no-prune-drop_1.0.0").is_dir());

    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "urn:kpar:exp-add-no-prune-drop"],
        cfg,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();

    // `--no-prune` must not remove the now-unneeded dependency from `.sysand`.
    run_sysand_in(
        &cwd,
        ["experimental", "add", "--dir", "new_dep", "--no-prune"],
        cfg,
    )?
    .assert()
    .success();

    assert!(env_lib.join("kpar.exp-add-no-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("b.exp_add_no_prune_new_1.0.0").is_dir());
    assert!(
        env_lib.join("kpar.exp-add-no-prune-drop_1.0.0").is_dir(),
        "`--no-prune` must leave the unneeded dependency installed in `.sysand`"
    );

    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    assert!(
        env_toml.contains("exp-add-no-prune-drop"),
        "`--no-prune` must leave the unneeded dependency registered in env.toml"
    );

    Ok(())
}

/// After `experimental remove` updates an existing lockfile, the lockfile
/// must remain internally consistent, and the dependency should be gone
/// from `.sysand`.
#[test]
fn exp_remove_keeps_lockfile_valid_and_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_lock_app", "1.0.0")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "b",
        Some("exp_remove_lock_dep"),
        Some("1.0.0"),
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

    run_sysand_in(&cwd, ["lock"], None)?.assert().success();
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("b.exp_remove_lock_dep_1.0.0").is_dir());

    run_sysand_in(
        &cwd,
        ["experimental", "remove", "b", "exp_remove_lock_dep"],
        None,
    )?
    .assert()
    .success();

    let lockfile =
        fs::read_to_string(cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME))?;
    assert!(
        !lockfile.contains("exp_remove_lock_dep"),
        "lockfile must not reference the removed dependency anywhere, including in the root's own usage list: {lockfile}"
    );

    assert!(
        !env_lib.join("b.exp_remove_lock_dep_1.0.0").exists(),
        "the dependency dropped by `experimental remove` must be pruned"
    );

    Ok(())
}

/// `experimental remove` must remove a project from `.sysand` once it is no
/// longer needed, while leaving dependencies that are still needed alone.
#[test]
fn exp_remove_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_prune_app", "1.0.0")?;
    out.assert().success();

    let keep_dir = cwd.join("keep_dep");
    fs::create_dir_all(&keep_dir)?;
    cli_init_project_in(
        &keep_dir,
        None,
        "b",
        Some("exp_remove_prune_keep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let drop_dir = cwd.join("drop_dep");
    fs::create_dir_all(&drop_dir)?;
    cli_init_project_in(
        &drop_dir,
        None,
        "b",
        Some("exp_remove_prune_drop"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let (_tmp_extra, cwd_extra, out) =
        cli_init_project_basic("a", "exp_remove_prune_extra", "1.0.0")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "keep_dep"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "drop_dep"],
        None,
    )?
    .assert()
    .success();

    // Install an unrelated project directly into the env, bypassing the
    // lockfile entirely, before any lockfile exists for this project.
    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:exp-remove-prune-extra",
            "--path",
            cwd_extra.as_str(),
        ],
        None,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("kpar.exp-remove-prune-extra_1.0.0").is_dir());

    // No lockfile has been generated yet, so this `experimental remove`
    // performs a full lock + sync of the remaining usages.
    run_sysand_in(
        &cwd,
        ["experimental", "remove", "b", "exp_remove_prune_drop"],
        None,
    )?
    .assert()
    .success();

    assert!(
        env_lib.join("b.exp_remove_prune_keep_1.0.0").is_dir(),
        "still-needed dependency must be installed"
    );
    assert!(
        !env_lib.join("kpar.exp-remove-prune-extra_1.0.0").exists(),
        "a project not present in the lockfile must be pruned from `.sysand` by default"
    );

    Ok(())
}

/// `experimental remove --no-prune` must leave a project that is not present
/// in the lockfile installed in `.sysand`, while still syncing dependencies
/// that are still needed.
#[test]
fn exp_remove_no_prune_keeps_unneeded_dependency_and_still_syncs()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_no_prune_app", "1.0.0")?;
    out.assert().success();

    let keep_dir = cwd.join("keep_dep");
    fs::create_dir_all(&keep_dir)?;
    cli_init_project_in(
        &keep_dir,
        None,
        "b",
        Some("exp_remove_no_prune_keep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let drop_dir = cwd.join("drop_dep");
    fs::create_dir_all(&drop_dir)?;
    cli_init_project_in(
        &drop_dir,
        None,
        "b",
        Some("exp_remove_no_prune_drop"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let (_tmp_extra, cwd_extra, out) =
        cli_init_project_basic("a", "exp_remove_no_prune_extra", "1.0.0")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "keep_dep"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "drop_dep"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:exp-remove-no-prune-extra",
            "--path",
            cwd_extra.as_str(),
        ],
        None,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");

    run_sysand_in(
        &cwd,
        [
            "experimental",
            "remove",
            "--no-prune",
            "b",
            "exp_remove_no_prune_drop",
        ],
        None,
    )?
    .assert()
    .success();

    assert!(
        env_lib.join("b.exp_remove_no_prune_keep_1.0.0").is_dir(),
        "`--no-prune` must not skip syncing dependencies that are still needed"
    );
    assert!(
        env_lib
            .join("kpar.exp-remove-no-prune-extra_1.0.0")
            .is_dir(),
        "`--no-prune` must leave a project not present in the lockfile installed in `.sysand`"
    );

    Ok(())
}
