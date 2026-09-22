// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The advisory lock from the CLI. The holder is this test process (`WorkspaceLock` from
//! `sysand_core`; `flock` conflicts across processes), the contender a
//! spawned `sysand`.
//!
//! Every `Utf8TempDir` outlives the lock it holds: on Windows a directory
//! with an open lock file cannot be removed.

use std::{error::Error, fs, thread, time::Duration};

use assert_cmd::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH},
    lock::{Lock, Lockfile, Project, Source, Usage},
    project::{ProjectRead as _, local_src::LocalSrcProject},
    utils::sha256_lowercase_hex,
    workspace_lock::{LockMode, WorkspaceLock},
};
use typed_path::Utf8UnixPathBuf;

mod common;
pub use common::*;

fn write_dep(dir: &Utf8Path, name: &str) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let model = format!("package {name} {{ }}\n");
    fs::write(dir.join(format!("{name}.sysml")), &model)?;
    fs::write(
        dir.join(".project.json"),
        format!(r#"{{"name":"{name}","publisher":"acme","version":"1.0.0"}}"#),
    )?;
    fs::write(
        dir.join(".meta.json"),
        format!(
            r#"{{"index":{{"{name}":"{name}.sysml"}},"created":"2026-01-01T00:00:00Z","checksum":{{"{name}.sysml":{{"algorithm":"SHA256","value":"{}"}}}}}}"#,
            sha256_lowercase_hex(&model)
        ),
    )?;
    Ok(())
}

/// An app in a fresh directory using `dep` from `deps/dep`, with a
/// lockfile but not yet synced.
fn locked_app() -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn Error>> {
    let (temp_dir, cwd) = new_temp_cwd()?;
    fs::write(
        cwd.join(".project.json"),
        r#"{"name":"app","publisher":"acme","version":"0.1.0","usage":[{"resource":"urn:dep","versionConstraint":"^1.0.0"}]}"#,
    )?;
    fs::write(
        cwd.join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )?;
    let dir = cwd.join("deps/dep");
    write_dep(&dir, "dep")?;
    let checksum = LocalSrcProject::new_access(&dir, None)
        .checksum_canonical_hex()?
        .expect("manifests");
    let lock = Lock {
        projects: vec![
            Project {
                publisher: Some("acme".to_owned()),
                name: "app".to_owned(),
                version: "0.1.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec![],
                usages: vec![Usage::from_str_unchecked("urn:dep")],
                sources: vec![Source::Editable {
                    editable: Utf8UnixPathBuf::from("."),
                }],
            },
            Project {
                publisher: Some("acme".to_owned()),
                name: "dep".to_owned(),
                version: "1.0.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec!["urn:dep".to_owned()],
                usages: vec![],
                sources: vec![Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("deps/dep"),
                    checksum,
                }],
            },
        ],
        ..Lock::default()
    };
    Lockfile::new(&cwd, lock).write()?;
    Ok((temp_dir, cwd))
}

fn held_text(root: &Utf8Path) -> String {
    format!("another process is using `{root}`")
}

#[test]
fn sync_fails_fast_while_locked() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let _held = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
    let out = run_sysand_in(&cwd, ["sync"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(held_text(&cwd)))
        .stderr(predicate::str::contains("Installing").not());
    // Nothing half-applied: the environment was never created; its
    // directory holds only the lock file.
    assert!(!cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH).exists());
    assert!(WorkspaceLock::path_for(&cwd).is_file());
    Ok(())
}

#[test]
fn lock_wait_then_deadline() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let _held = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
    let started = std::time::Instant::now();
    let out = run_sysand_in(&cwd, ["sync", "--lock-wait", "1"], None)?;
    assert!(started.elapsed() >= Duration::from_secs(1));
    out.assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "another process is using `{cwd}`; waiting up to 1 seconds"
        )))
        .stderr(predicate::str::contains(format!(
            "error: another process is using `{cwd}`"
        )));
    Ok(())
}

#[test]
fn lock_wait_succeeds_when_released() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let held = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
    let child = sysand_cmd_in(&cwd, ["sync", "--lock-wait", "5"], None)?
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    thread::sleep(Duration::from_millis(300));
    drop(held);
    let out = child.wait_with_output()?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("waiting up to 5 seconds"))
        .stderr(predicate::str::contains("Installing"));
    assert!(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH).is_file());
    Ok(())
}

#[test]
fn every_mutating_command_takes_the_lock() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    let _held = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
    // (`env install`/`env uninstall` are not exposed by the CLI.)
    let mutating: [&[&str]; 5] = [
        &["lock"],
        &["sync"],
        &["add", "--iri", "pkg:sysand/acme/other"],
        &["remove", "--iri", "urn:dep"],
        &["env"],
    ];
    for args in mutating {
        let out = run_sysand_in(&cwd, args.iter().copied(), None)?;
        out.assert()
            .failure()
            .stderr(predicate::str::contains(held_text(&cwd)));
    }
    // Readers and manifest-only commands run under a held lock.
    run_sysand_in(&cwd, ["info"], None)?.assert().success();
    run_sysand_in(&cwd, ["env", "list"], None)?
        .assert()
        .success();
    fs::create_dir_all(cwd.join("fresh"))?;
    run_sysand_in(
        &cwd.join("fresh"),
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "fresh",
            "--metamodel",
            "https://www.omg.org/spec/SysML/20250201",
        ],
        None,
    )?
    .assert()
    .success();
    Ok(())
}

#[test]
fn verify_takes_the_shared_lock() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    {
        let _exclusive = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
        run_sysand_in(&cwd, ["sync", "--verify"], None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains(held_text(&cwd)));
    }
    let _shared = WorkspaceLock::try_acquire(&cwd, LockMode::Shared)?;
    // A reader beside a reader.
    run_sysand_in(&cwd, ["sync", "--verify"], None)?
        .assert()
        .success()
        .stderr(
            predicate::str::contains("2 project(s) verified")
                .or(predicate::str::contains("1 project(s) verified")),
        );
    // A writer waits for the readers.
    run_sysand_in(&cwd, ["sync"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(held_text(&cwd)));
    run_sysand_in(&cwd, ["sync", "--repair"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(held_text(&cwd)));
    Ok(())
}

#[test]
fn verify_without_a_lockfile_locks_exclusively() -> Result<(), Box<dyn Error>> {
    // With no lockfile `sync` resolves and writes one first, so even
    // `--verify` must not run under a shared lock: a reader holding the
    // lock keeps it out.
    let (_temp_dir, cwd) = locked_app()?;
    fs::remove_file(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    let _shared = WorkspaceLock::try_acquire(&cwd, LockMode::Shared)?;
    run_sysand_in(&cwd, ["sync", "--verify"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(held_text(&cwd)));
    assert!(!cwd.join(DEFAULT_LOCKFILE_NAME).exists());
    Ok(())
}

/// Two `sync` processes on one project, started together, do not
/// interleave their writes.
#[test]
fn two_syncs_race() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    for round in 0..3 {
        // Both wait: both succeed, one after the other.
        let children: Vec<_> = std::iter::repeat_with(|| {
            sysand_cmd_in(&cwd, ["sync", "--lock-wait", "10"], None)?
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(Into::into)
        })
        .take(2)
        .collect::<Result<_, Box<dyn Error>>>()?;
        for child in children {
            child.wait_with_output()?.assert().success();
        }
        run_sysand_in(&cwd, ["sync", "--verify"], None)?
            .assert()
            .success();

        // Neither waits: any loser fails with the lock-held error and
        // leaves the environment coherent.
        fs::remove_dir_all(cwd.join(DEFAULT_ENV_NAME))?;
        let children: Vec<_> = std::iter::repeat_with(|| {
            sysand_cmd_in(&cwd, ["sync"], None)?
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(Into::into)
        })
        .take(2)
        .collect::<Result<_, Box<dyn Error>>>()?;
        let mut successes = 0;
        for child in children {
            let out = child.wait_with_output()?;
            if out.status.success() {
                successes += 1;
            } else {
                assert!(
                    String::from_utf8_lossy(&out.stderr).contains(&held_text(&cwd)),
                    "round {round}: a loser fails only with the lock-held error: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
        assert!(successes >= 1, "round {round}: one writer always wins");
        run_sysand_in(&cwd, ["sync", "--verify"], None)?
            .assert()
            .success();
    }
    Ok(())
}
