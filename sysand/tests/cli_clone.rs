// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use assert_cmd::prelude::*;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use predicates::prelude::*;
use sysand_core::{env::local_directory::DEFAULT_ENV_NAME, project::utils::wrapfs};

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
    assert!(target.join(env_path).join("entries.txt").is_file());
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
    assert!(!target.join(env_path).join("entries.txt").exists());
}

/// `p` must be absolute OS-native path
fn file_url_from_path(p: impl AsRef<Path>) -> String {
    url::Url::from_file_path(p).unwrap().to_string()
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
    let (_temp_dir, cwd, out) = run_sysand(["clone", &file_url, "-v"], None)?;

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

    // TODO: test error message
    out.assert().failure();
    assert_dir_empty(&cwd)?;

    // explicit path
    let (_temp_dir, cwd, out) = run_sysand(["clone", "--path", "../../does/not/exist"], None)?;

    out.assert().failure();
    assert_dir_empty(&cwd)?;

    Ok(())
}

// clone chooses latest version if none given
// #[test]
// fn clone_choose_latest_version() -> Result<(), Box<dyn std::error::Error>> {
//     // will need mock index server here
//     todo!()
// }

// warn if deps of cloned project include std libs
// #[test]
// fn clone_std_warn() -> Result<(), Box<dyn std::error::Error>> {
//     todo!()
// }

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
