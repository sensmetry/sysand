// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use mockito::Matcher;
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
    std::fs::write(
        proj_dir.join(".project.json"),
        r#"{
  "name": "sync_to_local",
  "version": "1.2.3",
  "usage": []
}
"#,
    )?;
    std::fs::write(
        proj_dir.join(".meta.json"),
        r#"{
  "index": {},
  "created": "2025-06-12T10:48:55.597880Z"
}
"#,
    )?;

    std::fs::write(
        cwd.join(DEFAULT_LOCKFILE_NAME),
        r#"lock_version = "0.3"

[[project]]
name = "sync_to_local"
version = "1.2.3"
identifiers = ["urn:kpar:sync_to_local"]
checksum = "4b3adfb7bea950c7c598093c50323fa2ea9f816cb4b10cd299b205bfd4b47a5c"
sources = [
    { src_path = "lib/sync_to_local" },
]
"#,
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
        .stdout(predicate::str::contains("`urn:kpar:sync_to_local` 1.2.3"));

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("env is already up to date"));

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

    std::fs::write(
        cwd.join(DEFAULT_LOCKFILE_NAME),
        format!(
            r#"lock_version = "0.3"

[[project]]
name = "sync_to_remote"
version = "1.2.3"
identifiers = ["urn:kpar:sync_to_remote"]
checksum = "39f49107a084ab27624ee78d4d37f87a1f7606a2b5d242cdcd9374cf20ab1895"
sources = [
    {{ remote_src = "{}" }},
]
"#,
            &server.url()
        ),
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
        .stdout(predicate::str::contains("`urn:kpar:sync_to_remote` 1.2.3"));

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("env is already up to date"));

    Ok(())
}

#[test]
fn sync_to_remote_auth() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;

    let mut server = mockito::Server::new();

    let info_mock = server
        .mock("GET", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"sync_to_remote","version":"1.2.3","usage":[]}"#)
        .expect(4) // TODO: Reduce this to 1
        .create();

    let info_mock_auth = server
        .mock("GET", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"sync_to_remote","version":"1.2.3","usage":[]}"#)
        .expect(4) // TODO: Reduce this to 1
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(4) // TODO: Reduce this to 1
        .create();

    let meta_mock_auth = server
        .mock("GET", "/.meta.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(4) // TODO: Reduce this to 1
        .create();

    std::fs::write(
        cwd.join(DEFAULT_LOCKFILE_NAME),
        format!(
            r#"lock_version = "0.3"

[[project]]
name = "sync_to_remote"
version = "1.2.3"
identifiers = ["urn:kpar:sync_to_remote"]
checksum = "39f49107a084ab27624ee78d4d37f87a1f7606a2b5d242cdcd9374cf20ab1895"
sources = [
    {{ remote_src = "{}" }},
]
"#,
            &server.url()
        )
        .as_bytes(),
    )?;

    let out = run_sysand_in_with(
        &cwd,
        ["sync"],
        None,
        &IndexMap::from([
            (
                "SYSAND_CRED_TEST",
                format!("http://{}/**", server.host_with_port()).as_str(),
            ),
            ("SYSAND_CRED_TEST_BASIC_USER", "user_1234"),
            ("SYSAND_CRED_TEST_BASIC_PASS", "pass_4321"),
        ]),
    )?;

    info_mock.assert();
    info_mock_auth.assert();
    meta_mock.assert();
    meta_mock_auth.assert();

    out.assert()
        .success()
        .stderr(predicate::str::contains("Creating"))
        .stderr(predicate::str::contains("Syncing"))
        .stderr(predicate::str::contains("Installing"));

    let out = run_sysand_in(&cwd, ["env", "list"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("`urn:kpar:sync_to_remote` 1.2.3"));

    let out = run_sysand_in(&cwd, ["sync"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("env is already up to date"));

    Ok(())
}

#[test]
fn sync_to_remote_incorrect_auth() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;

    let mut server = mockito::Server::new();

    let info_mock = server
        .mock("GET", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"sync_to_remote","version":"1.2.3","usage":[]}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let info_mock_auth = server
        .mock("GET", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"sync_to_remote","version":"1.2.3","usage":[]}"#)
        .expect(0) // TODO: Reduce this to 1
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_auth = server
        .mock("GET", "/.meta.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(0) // TODO: Reduce this to 1
        .create();

    std::fs::write(
        cwd.join(DEFAULT_LOCKFILE_NAME),
        format!(
            r#"lock_version = "0.3"

[[project]]
name = "sync_to_remote"
version = "1.2.3"
identifiers = ["urn:kpar:sync_to_remote"]
checksum = "39f49107a084ab27624ee78d4d37f87a1f7606a2b5d242cdcd9374cf20ab1895"
sources = [
    {{ remote_src = "{}" }},
]
"#,
            &server.url()
        ),
    )?;

    let out = run_sysand_in_with(
        &cwd,
        ["sync"],
        None,
        &IndexMap::from([
            ("SYSAND_CRED_TEST", "http://127.0.0.1:80/**"),
            ("SYSAND_CRED_TEST_BASIC_USER", "user_1234"),
            ("SYSAND_CRED_TEST_BASIC_PASS", "pass_4321"),
        ]),
    )?;

    info_mock.assert();
    info_mock_auth.assert();
    meta_mock.assert();
    meta_mock_auth.assert();

    out.assert().failure();

    Ok(())
}
