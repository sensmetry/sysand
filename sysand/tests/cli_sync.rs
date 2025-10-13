// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Write;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn sync_to_local() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;

    // Create local project that can be referred to with src_path
    let lib_dir = cwd.join("lib");
    std::fs::create_dir(&lib_dir)?;
    let proj_dir = lib_dir.join("sync_to_local");
    std::fs::create_dir(&proj_dir)?;
    let mut info_file = std::fs::File::create_new(proj_dir.join(".project.json"))?;
    info_file.write_all(
        r#"{
  "name": "sync_to_local",
  "version": "1.2.3",
  "usage": []
}
"#
        .as_bytes(),
    )?;
    let mut meta_file = std::fs::File::create_new(proj_dir.join(".meta.json"))?;
    meta_file.write_all(
        r#"{
  "index": {},
  "created": "2025-06-12T10:48:55.597880Z"
}
"#
        .as_bytes(),
    )?;

    let mut lockfile = std::fs::File::create_new(cwd.join(DEFAULT_LOCKFILE_NAME))?;

    lockfile.write_all(
        r#"lock_version = "0.1"

[[project]]
name = "sync_to_local"
version = "1.2.3"
iris = ["urn:kpar:sync_to_local"]
checksum = "4b3adfb7bea950c7c598093c50323fa2ea9f816cb4b10cd299b205bfd4b47a5c"
sources = [
    { src_path = "lib/sync_to_local" },
]
"#
        .as_bytes(),
    )?;

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Creating"))
        .stderr(predicate::str::contains("Syncing"))
        .stderr(predicate::str::contains("Installing"));

    let out = run_sysand_in(&cwd, ["env", "list"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("urn:kpar:sync_to_local 1.2.3"));

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("env is up to date"));

    Ok(())
}

#[test]
fn sync_to_remote() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;

    let mut server = mockito::Server::new();

    let info_mock = server
        .mock("GET", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"sync_to_remote","version":"1.2.3","usage":[]}"#)
        .expect_at_most(4) // TODO: Reduce this to 1 after caching
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect_at_most(4) // TODO: Reduce this to 1 after caching
        .create();

    let mut lockfile = std::fs::File::create_new(cwd.join(DEFAULT_LOCKFILE_NAME))?;

    lockfile.write_all(
        format!(
            r#"lock_version = "0.1"

[[project]]
name = "sync_to_remote"
version = "1.2.3"
iris = ["urn:kpar:sync_to_remote"]
checksum = "39f49107a084ab27624ee78d4d37f87a1f7606a2b5d242cdcd9374cf20ab1895"
sources = [
    {{ remote_src = "{}" }},
]
"#,
            &server.url()
        )
        .as_bytes(),
    )?;

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("Creating"))
        .stderr(predicate::str::contains("Syncing"))
        .stderr(predicate::str::contains("Installing"));

    info_mock.assert();
    meta_mock.assert();

    let out = run_sysand_in(&cwd, ["env", "list"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("urn:kpar:sync_to_remote 1.2.3"));

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("env is up to date"));

    Ok(())
}
