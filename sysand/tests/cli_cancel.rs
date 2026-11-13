// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Cancellation from the CLI: Ctrl-C during a
//! stalled download is observed at the next bridge, the run exits 130,
//! and the workspace is left in a state `sync --verify` classifies.
//!
//! The "index" is a listener that accepts and never answers, so the
//! download stalls until the signal lands. Unix only: the test sends
//! `SIGINT` itself.

#![cfg(unix)]

use std::{error::Error, fs, net::TcpListener, thread, time::Duration};

use assert_cmd::prelude::*;
use camino::Utf8PathBuf;
use camino_tempfile::Utf8TempDir;
use nix::{sys::signal, unistd::Pid};
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH},
    lock::{Lock, Lockfile, Project, Source, Usage},
};
use typed_path::Utf8UnixPathBuf;

mod common;
pub use common::*;

/// A server that accepts connections and never answers them.
fn tarpit() -> Result<(String, thread::JoinHandle<()>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}/", listener.local_addr()?);
    let handle = thread::spawn(move || {
        // Kept open and never answered until the listener goes away.
        let mut held = Vec::new();
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => held.push(stream),
                Err(_) => break,
            }
        }
        eprintln!("tarpit held {} connection(s)", held.len());
    });
    Ok((url, handle))
}

/// An app whose one dependency is an index archive at `url`.
fn app_locked_against(url: &str) -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn Error>> {
    let (temp_dir, cwd) = new_temp_cwd()?;
    fs::write(
        cwd.join(".project.json"),
        r#"{"name":"app","publisher":"acme","version":"0.1.0","usage":[{"resource":"pkg:sysand/mock/dep","versionConstraint":"^1.0.0"}]}"#,
    )?;
    fs::write(
        cwd.join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )?;
    let lock = Lock {
        projects: vec![
            Project {
                publisher: Some("acme".to_owned()),
                name: "app".to_owned(),
                version: "0.1.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec![],
                usages: vec![Usage::from_str_unchecked("pkg:sysand/mock/dep")],
                sources: vec![Source::Editable {
                    editable: Utf8UnixPathBuf::from("."),
                }],
            },
            Project {
                publisher: Some("mock".to_owned()),
                name: "dep".to_owned(),
                version: "1.0.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec!["pkg:sysand/mock/dep".to_owned()],
                usages: vec![],
                sources: vec![Source::IndexKpar {
                    index_kpar: fluent_uri::Iri::parse(format!("{url}mock/dep/1.0.0/project.kpar"))
                        .expect("a mock index URL"),
                    kpar_size: std::num::NonZeroU64::new(42).unwrap(),
                    kpar_digest: "a".repeat(64),
                }],
            },
        ],
        ..Lock::default()
    };
    Lockfile::new(&cwd, lock).write()?;
    Ok((temp_dir, cwd))
}

fn interrupt_after(child: &std::process::Child, after: Duration) {
    thread::sleep(after);
    signal::kill(
        Pid::from_raw(child.id().cast_signed()),
        signal::Signal::SIGINT,
    )
    .unwrap();
}

#[test]
fn ctrl_c_cancels_a_stalled_download() -> Result<(), Box<dyn Error>> {
    let (url, _server) = tarpit()?;
    let (_temp_dir, cwd) = app_locked_against(&url)?;
    let child = sysand_cmd_in(&cwd, ["sync", "--default-index", &url], None)?
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    interrupt_after(&child, Duration::from_millis(700));
    let out = child.wait_with_output()?;
    assert_eq!(
        out.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.assert()
        .stderr(predicate::str::contains(
            "cancelling; the step in flight finishes first",
        ))
        .stderr(predicate::str::contains("error: cancelled"));

    // The environment holds nothing half-written: `--verify` classifies
    // the state (the dependency is simply not installed), and repair is
    // an ordinary sync once the index answers.
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert().failure().stderr(predicate::str::contains(
        "`pkg:sysand/mock/dep` 1.0.0 is in `sysand-lock.toml` but not installed",
    ));
    Ok(())
}

#[test]
fn ctrl_c_during_lock_leaves_no_lockfile() -> Result<(), Box<dyn Error>> {
    // The lockfile is written whole at the end of a solve; a cancel that
    // lands while the solve waits on the index leaves no lockfile at all.
    let (url, _server) = tarpit()?;
    let (_temp_dir, cwd) = app_locked_against(&url)?;
    fs::remove_file(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    let child = sysand_cmd_in(&cwd, ["lock", "--default-index", &url], None)?
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    interrupt_after(&child, Duration::from_millis(700));
    let out = child.wait_with_output()?;
    assert_eq!(
        out.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!cwd.join(DEFAULT_LOCKFILE_NAME).exists());
    assert!(!cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH).exists());
    Ok(())
}

#[test]
fn a_second_ctrl_c_exits_at_once() -> Result<(), Box<dyn Error>> {
    // The first Ctrl-C cancels and waits for the step in flight; a
    // stalled connection is such a step until its bridge returns, so the
    // second Ctrl-C is the way out at once.
    let (url, _server) = tarpit()?;
    let (_temp_dir, cwd) = app_locked_against(&url)?;
    let child = sysand_cmd_in(&cwd, ["sync", "--default-index", &url], None)?
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    interrupt_after(&child, Duration::from_millis(700));
    interrupt_after(&child, Duration::from_millis(200));
    let out = child.wait_with_output()?;
    assert_eq!(out.status.code(), Some(130));
    Ok(())
}

#[test]
fn a_plain_error_is_still_exit_1() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let out = run_sysand_in(&cwd, ["sync"], None)?;
    assert_eq!(out.status.code(), Some(1));
    Ok(())
}
