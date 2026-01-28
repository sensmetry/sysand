// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use assert_cmd::prelude::*;
use camino::Utf8Path;
use mockito::Server;
use predicates::prelude::*;
use sysand_core::env::local_directory::DEFAULT_ENV_NAME;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// sysand env should create an empty local environment in
/// ./sysand_env, containing only an empty entries.txt file
#[test]
fn env_init_empty_env() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(["env"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let env_path = Path::new(DEFAULT_ENV_NAME);

    for entry in std::fs::read_dir(&cwd)? {
        let path = entry?.path();

        assert!(path.is_dir() || path.is_file());

        if path.is_dir() {
            assert_eq!(path.strip_prefix(&cwd)?, env_path);
        } else {
            // if path.is_file()
            assert_eq!(path.strip_prefix(&cwd)?, env_path.join("entries.txt"));
        }
    }

    assert_eq!(
        std::fs::File::open(cwd.join("sysand_env/entries.txt"))?
            .metadata()?
            .len(),
        0
    );

    Ok(())
}

/// `sysand env install <IRI> --location <LOCATION>` should install the
/// interchange project at <LOCATION> as <IRI> in local env, `sysand env
/// list` should print the <IRI> and version to stdout and `sysand env
/// uninstall <IRI>` should remove it from the env
#[test]
fn env_install_from_local_dir() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) = run_sysand(["env"], None)?;

    let test_path = fixture_path("test_lib");

    let env_path = Utf8Path::new(DEFAULT_ENV_NAME);

    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:test",
            "--path",
            test_path.as_str(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("`urn:kpar:test` 0.0.1"));

    assert_eq!(
        std::fs::read_to_string(cwd.join(env_path).join("entries.txt"))?,
        "urn:kpar:test\n"
    );

    let test_hash = "fdfa3ca7927959186c3b55733ea3a7fa00a42fd7dca48365c5529054ff78358b";

    assert!(cwd.join(env_path).join(test_hash).is_dir());

    assert_eq!(
        std::fs::read_to_string(cwd.join(env_path).join(test_hash).join("versions.txt"))?,
        "0.0.1\n"
    );

    assert!(
        cwd.join(env_path)
            .join(test_hash)
            .join("0.0.1.kpar")
            .is_dir()
    );

    assert!(
        cwd.join(env_path)
            .join(test_hash)
            .join("0.0.1.kpar")
            .join(".project.json")
            .is_file()
    );

    assert!(
        cwd.join(env_path)
            .join(test_hash)
            .join("0.0.1.kpar")
            .join(".meta.json")
            .is_file()
    );

    let out = run_sysand_in(&cwd, ["env", "list"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("`urn:kpar:test` 0.0.1"));

    let out = run_sysand_in(&cwd, ["env", "uninstall", "urn:kpar:test"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("urn:kpar:test"));

    let entries = std::fs::read_dir(cwd.join(env_path))?.collect::<Result<Vec<_>, _>>()?;

    assert_eq!(entries.len(), 1);

    assert_eq!(entries[0].file_name(), "entries.txt");

    assert_eq!(std::fs::read_to_string(entries[0].path())?, "");

    Ok(())
}

#[test]
fn env_install_from_http_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) = run_sysand(["env"], None)?;

    let test_path = fixture_path("test_lib.kpar");

    let env_path = Utf8Path::new(DEFAULT_ENV_NAME);

    let mut server = Server::new();

    let test_body = std::fs::read(test_path)?;

    let _git_mock = server
        .mock("GET", "/test_lib.kpar/info/refs?service=git-upload-pack")
        .with_status(404)
        .expect_at_least(0)
        .create();

    let _project_mock = server
        .mock("HEAD", "/test_lib.kpar/.project.json")
        .with_status(404)
        .expect_at_least(0)
        .create();

    let _meta_mock = server
        .mock("HEAD", "/test_lib.kpar/.meta.json")
        .with_status(404)
        .expect_at_least(0)
        .create();

    let head_mock = server
        .mock("HEAD", "/test_lib.kpar")
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(&test_body)
        .expect_at_least(0)
        .create();

    let get_mock = server
        .mock("GET", "/test_lib.kpar")
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(&test_body)
        .expect_at_least(1)
        .create();

    let project_url = format!("{}/test_lib.kpar", server.url());

    let out = run_sysand_in(&cwd, ["env", "install", &project_url], None)?;

    head_mock.assert();
    get_mock.assert();

    out.assert().success();

    assert_eq!(
        std::fs::read_to_string(cwd.join(env_path).join("entries.txt"))?,
        format!("{}\n", &project_url)
    );

    Ok(())
}

/// `sysand env install <IRI> --location <LOCATION>` should install
/// the interchange project att <LOCATION> as <IRI> in local env.
/// If the same command is run again it should give an error,
/// and if run again with flag `--allow-overwrite` it should succeed
#[test]
fn env_install_from_local_dir_allow_overwrite() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) = run_sysand(["env"], None)?;

    let test_path = fixture_path("test_lib");

    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:test",
            "--path",
            test_path.as_str(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("`urn:kpar:test` 0.0.1"));

    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:test",
            "--path",
            test_path.as_str(),
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("error"));

    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:test",
            "--path",
            test_path.as_str(),
            "--allow-overwrite",
        ],
        None,
    )?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("`urn:kpar:test` 0.0.1"));

    Ok(())
}

// TODO: Write helper function to generate an index and add tests for
// installing from index and for using flag '--allow-multiple'.
