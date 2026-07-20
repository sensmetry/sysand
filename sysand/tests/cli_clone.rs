// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::path::Path;

use assert_cmd::prelude::*;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use predicates::prelude::*;
use sysand_core::{env::DEFAULT_ENV_NAME, project::utils::wrapfs};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

// TODO: add to test data:
// - invalid project (e.g. missing included file)
// - project that depends on std libs
// - project that depends on another project

fn assert_libtest_cloned_synced(target: impl AsRef<Path>) {
    let env_path = Path::new(DEFAULT_ENV_NAME);
    let target = target.as_ref();

    assert!(target.join(".project.json").is_file());
    assert!(target.join(".meta.json").is_file());
    assert!(target.join("sysand-lock.toml").is_file());
    assert!(target.join("libtest.sysml").is_file());
    assert!(target.join("extras").is_dir());
    assert!(target.join(env_path).is_dir());
    assert!(target.join(env_path).join("env.toml").is_file());
}

fn assert_only_libtest_cloned(target: impl AsRef<Path>) {
    let env_path = Path::new(DEFAULT_ENV_NAME);
    let target = target.as_ref();

    assert!(target.join(".project.json").is_file());
    assert!(target.join(".meta.json").is_file());
    assert!(target.join("libtest.sysml").is_file());
    assert!(target.join("extras").is_dir());
    assert!(!target.join("sysand-lock.toml").exists());
    assert!(!target.join(env_path).exists());
    assert!(!target.join(env_path).join("env.toml").exists());
}

/// Assert that the given path is an empty dir
fn assert_dir_empty(p: impl AsRef<Utf8Path>) -> Result<(), Box<dyn std::error::Error>> {
    let mut dir_it = wrapfs::read_dir(p)?;
    assert!(dir_it.next().is_none());
    Ok(())
}

// clone project from path locator, explicit path, `file`
// iri locator or explicit `file` iri
// should clone the project into cwd (it's the default target),
// create lockfile and env
#[test]
fn clone_project_default_target() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = fixture_path("test_lib");
    let test_path_str = test_path.as_str();
    // auto path form locator
    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    // explicit path
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--path", test_path_str], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    let file_url = file_url_from_path(&test_path);
    // auto path from `file` iri
    let (_temp_dir, cwd, out) = run_sysand(["clone", &file_url], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    // explicit `file` iri
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--iri", &file_url], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    Ok(())
}

#[test]
fn clone_local_kpar_project_default_target() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = fixture_path("test_lib.kpar");
    let test_path_str = test_path.as_str();

    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    let (_temp_dir, cwd, out) = run_sysand(["clone", "--path", test_path_str], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(&cwd);

    Ok(())
}

// clone remote project
// #[test]
// fn clone_remote_project() -> Result<(), Box<dyn std::error::Error>> {
//     // needs mock index
//     todo!()
// }

// clone fail when wrong version given for local project
#[test]
fn clone_wrong_version() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = fixture_path("test_lib");
    let test_path_str = test_path.as_str();
    // auto path form locator
    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str, "--version", "0.0.2"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "given version 0.0.2 does not match project version",
    ));
    assert_dir_empty(&cwd)?;

    // explicit path
    let (_temp_dir, cwd, out) = run_sysand(
        ["clone", "--path", test_path_str, "--version", "0.0.2"],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "given version 0.0.2 does not match project version",
    ));
    assert_dir_empty(&cwd)?;

    let file_url = file_url_from_path(&test_path);
    // auto path from `file` iri
    let (_temp_dir, cwd, out) = run_sysand(["clone", &file_url, "--version", "0.0.2"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));
    assert_dir_empty(&cwd)?;

    // explicit `file` iri
    let (_temp_dir, cwd, out) =
        run_sysand(["clone", "--iri", &file_url, "--version", "0.0.2"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));
    assert_dir_empty(&cwd)?;

    Ok(())
}

// clone fail when project was not found
#[test]
fn clone_not_found() -> Result<(), Box<dyn std::error::Error>> {
    // Directory exists, but does not contain project
    let test_path = fixture_path("");
    let test_path_str = test_path.as_str();
    // auto path form locator
    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "incomplete project: missing `.project.json` and `.meta.json`",
    ));
    assert_dir_empty(&cwd)?;

    // explicit path
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--path", "../../does/not/exist"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("failed to canonicalize path"));
    assert_dir_empty(&cwd)?;

    Ok(())
}

// clone chooses latest version if none given
// #[test]
// fn clone_choose_latest_version() -> Result<(), Box<dyn std::error::Error>> {
//     // will need mock index server here
//     todo!()
// }

// note if deps of cloned project include std libs
#[test]
fn clone_std_deps_note() -> Result<(), Box<dyn std::error::Error>> {
    let (_dep_temp_dir, cwd_dep, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "clone_std_note_dep"],
        None,
    )?;
    out.assert().success();

    std::fs::write(
        cwd_dep.join("CloneStdNoteDep.sysml"),
        "package CloneStdNoteDep;",
    )?;
    run_sysand_in(&cwd_dep, ["include", "CloneStdNoteDep.sysml"], None)?
        .assert()
        .success();

    run_sysand_in(
        &cwd_dep,
        [
            "add",
            "--no-lock",
            "--include-std",
            "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
        ],
        None,
    )?
    .assert()
    .success();

    let dep_path_str = cwd_dep.as_str();
    let (_temp_dir, cwd, out) = run_sysand(["clone", dep_path_str], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "note: SysMLv2/KerML standard library packages will not be installed during sync,",
        ))
        .stderr(predicate::str::contains("run `sysand sync --include-std`"));

    assert!(cwd.join(DEFAULT_ENV_NAME).is_dir());

    Ok(())
}

// do not warn about std deps if `--include-std` is given
// TODO: should also check that they are installed (mock them)
// #[test]
// fn clone_include_std() -> Result<(), Box<dyn std::error::Error>> {
//     todo!()
// }

// if `--no-deps` is given, lockfile and env won't be created
#[test]
fn clone_no_deps() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = fixture_path("test_lib");
    let test_path_str = test_path.as_str();
    // auto path form locator
    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str, "--no-deps"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_only_libtest_cloned(&cwd);

    // explicit path
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--path", test_path_str, "--no-deps"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_only_libtest_cloned(&cwd);

    let file_url = file_url_from_path(&test_path);
    // auto path from `file` iri
    let (_temp_dir, cwd, out) = run_sysand(["clone", &file_url, "--no-deps"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_only_libtest_cloned(&cwd);

    // explicit `file` iri
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--iri", &file_url, "--no-deps"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_only_libtest_cloned(&cwd);

    Ok(())
}

// clone cleans up on fail when cloning corrupt project
// #[test]
// fn clone_cleanup() -> Result<(), Box<dyn std::error::Error>> {
//     todo!()
// }

// clone fails when target is not empty
// target contents are untouched
#[test]
fn clone_non_empty_target() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempdir()?;
    let path = tmp.path();
    let file = path.join("test.txt");
    wrapfs::write(&file, "abc123")?;
    let out = run_sysand_in(path, ["clone", "urn:kpar:does-not-matter"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("target directory not empty"));
    assert!(file.exists());

    Ok(())
}

// clone works when a nonexistent (also possibly nested) target is given
#[test]
fn clone_nonexsitent_nested_target() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = fixture_path("test_lib");
    let test_path_str = test_path.as_str();
    let target = "path/to/target/dir";
    // auto path form locator
    let (_temp_dir, cwd, out) = run_sysand(["clone", test_path_str, "--target", target], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Cloned `Lib test` 0.0.1"));
    assert_libtest_cloned_synced(cwd.join(target));

    Ok(())
}
