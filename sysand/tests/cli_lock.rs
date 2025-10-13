// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::{Path, PathBuf};

use assert_cmd::prelude::*;
use mockito::{Server, ServerGuard};
use predicates::{prelude::*, str::contains};
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::local_directory::{DEFAULT_ENV_NAME, ENTRIES_PATH},
    lock::{Lock, Source},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;
use serde_json::json;

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the current working directory. (Non-interactive use)
#[test]
fn lock_trivial() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--name", "lock_trivial", "--version", "1.2.3"],
        None,
    )?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["lock"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(projects.len(), 1);

    let project = &projects[0];

    assert_eq!(project.sources.len(), 1);

    let Source::Editable { editable } = &project.sources[0] else {
        panic!();
    };

    assert_eq!(PathBuf::from("."), PathBuf::from(editable));

    Ok(())
}

fn mock_project<
    P: AsRef<str>,
    N: AsRef<str>,
    V: AsRef<str>,
    U: AsRef<str>,
    I: IntoIterator<Item = U>,
>(
    server: &mut ServerGuard,
    path: P,
    name: N,
    version: V,
    deps: I,
) -> String {
    let usage: Vec<serde_json::Value> = deps
        .into_iter()
        .map(|dep| json!({"resource": dep.as_ref()}))
        .collect();

    server
        .mock("HEAD", format!("/{}/.project.json", path.as_ref()).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"name": name.as_ref(), "version": version.as_ref(), "usage": usage}).to_string(),
        )
        .create();

    server
        .mock("GET", format!("/{}/.project.json", path.as_ref()).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"name": name.as_ref(), "version": version.as_ref(), "usage": usage}).to_string(),
        )
        .create();

    server
        .mock("HEAD", format!("/{}/.meta.json", path.as_ref()).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"index":{}, "created": "0000-00-00T00:00:00.123456789Z"}).to_string())
        .create();

    server
        .mock("GET", format!("/{}/.meta.json", path.as_ref()).as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"index":{}, "created": "0000-00-00T00:00:00.123456789Z"}).to_string())
        .create();

    format!("{}/{}", server.url(), path.as_ref())
}

const NO_DEP: [&str; 0] = [""; 0];

fn inject_usages<P: AsRef<Path>, D: AsRef<str>, I: IntoIterator<Item = D>>(
    project_path: P,
    usages: I,
) -> Result<(), Box<dyn std::error::Error>> {
    inject_usages_versions(
        project_path,
        usages.into_iter().map(|x| -> (D, Option<D>) { (x, None) }),
    )
}

fn inject_usages_versions<
    P: AsRef<Path>,
    D: AsRef<str>,
    VR: AsRef<str>,
    I: IntoIterator<Item = (D, Option<VR>)>,
>(
    project_path: P,
    usages: I,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut info: InterchangeProjectInfoRaw = serde_json::from_str::<InterchangeProjectInfoRaw>(
        &std::fs::read_to_string(&project_path)?,
    )?;

    for (usage, version_req) in usages {
        info.usage.push(InterchangeProjectUsageRaw {
            resource: usage.as_ref().to_string(),
            version_constraint: version_req.map(|x| x.as_ref().to_string()),
        });
    }

    std::fs::write(&project_path, serde_json::to_string(&info)?)?;

    Ok(())
}

#[test]
fn lock_basic_http_deps() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::new();

    let c_url = mock_project(&mut server, "c", "lock_basic_http_deps_c", "1.0.0", NO_DEP);

    let a_url = mock_project(
        &mut server,
        "a",
        "lock_basic_http_deps_a",
        "1.0.0",
        [&c_url],
    );
    let b_url = mock_project(
        &mut server,
        "b",
        "lock_basic_http_deps_b",
        "1.0.0",
        [&c_url],
    );

    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--name",
            "lock_basic_http_deps",
            "--version",
            "1.2.3",
        ],
        None,
    )?;

    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages(cwd.join(".project.json"), [a_url.clone(), b_url.clone()])?;

    let out = run_sysand_in(&cwd, ["lock"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(projects.len(), 4);

    let project_names: Vec<_> = projects
        .iter()
        .cloned()
        .map(|project| project.name)
        .collect();

    assert!(project_names.contains(&"lock_basic_http_deps".to_string()));
    assert!(project_names.contains(&"lock_basic_http_deps_a".to_string()));
    assert!(project_names.contains(&"lock_basic_http_deps_b".to_string()));
    assert!(project_names.contains(&"lock_basic_http_deps_c".to_string()));

    run_sysand_in(&cwd, ["env"], None)?
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    run_sysand_in(&cwd, ["sync"], None)?
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let entries: Vec<String> =
        std::fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(ENTRIES_PATH))?
            .lines()
            .map(|x| x.to_string())
            .collect();

    assert_eq!(entries.len(), 3);

    assert!(entries.contains(&a_url));
    assert!(entries.contains(&b_url));
    assert!(entries.contains(&c_url));

    Ok(())
}

#[test]
fn lock_fail_unsatisfiable() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::new();

    let a_url = mock_project(&mut server, "a", "lock_basic_http_deps_a", "1.0.0", NO_DEP);

    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--name",
            "lock_basic_http_deps",
            "--version",
            "1.2.3",
        ],
        None,
    )?;

    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages_versions(cwd.join(".project.json"), [(a_url, Some(">1.0.0"))])?;

    let out = run_sysand_in(&cwd, ["lock"], None)?;

    out.assert()
        .failure()
        .stderr(contains("Failed to satisfy usage constraints"));

    Ok(())
}
