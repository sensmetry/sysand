// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::path::Path;

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

    assert_eq!(".", editable.as_str());

    Ok(())
}

#[test]
fn lock_local_source() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--name", "lock_local_source", "--version", "1.2.3"],
        None,
    )?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["init", "--version", "1.0.0", "local_dep"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["add", "urn:kpar:local_dep", "--no-lock"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let cfg = toml::to_string(&sysand_core::config::Config {
        indexes: vec![],
        projects: vec![sysand_core::config::ConfigProject {
            identifiers: vec!["urn:kpar:local_dep".to_string()],
            sources: vec![sysand_core::lock::Source::LocalSrc {
                src_path: cwd.join("local_dep").as_str().into(),
            }],
        }],
    })?;

    let cfg_path = cwd.join(sysand_core::config::local_fs::CONFIG_FILE);
    std::fs::write(&cfg_path, cfg)?;

    let out = run_sysand_in(&cwd, ["lock"], Some(cfg_path.as_str()))?;

    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(projects.len(), 2);

    Ok(())
}

#[test]
fn lock_std_lib() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--name", "lock_std_lib", "--version", "1.2.3"],
        None,
    )?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "urn:kpar:function-library",
            "--no-lock",
            "--include-std",
        ],
        None,
    )?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `urn:kpar:function-library`",
    ));

    let cfg = toml::to_string(&sysand_core::config::Config {
        indexes: vec![],
        projects: vec![sysand_core::config::ConfigProject {
            identifiers: vec!["urn:kpar:local_dep".to_string()],
            sources: vec![sysand_core::lock::Source::LocalSrc {
                src_path: cwd.join("local_dep").as_str().into(),
            }],
        }],
    })?;

    let cfg_path = cwd.join(sysand_core::config::local_fs::CONFIG_FILE);
    std::fs::write(&cfg_path, cfg)?;

    let out = run_sysand_in(&cwd, ["lock"], Some(cfg_path.as_str()))?;

    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(projects.len(), 4);

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
        .filter_map(|project| project.name)
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

/// Build a minimal valid kpar (ZIP) archive carrying the required
/// `root/.project.json` and `root/.meta.json` entries. The project-digest
/// written by the caller to `versions.json` must match the canonical hash
/// of those entries or `sysand sync` will (correctly) reject the download.
fn build_index_kpar_bytes(
    name: &str,
    version: &str,
) -> (
    Vec<u8>,
    sysand_core::model::InterchangeProjectInfoRaw,
    sysand_core::model::InterchangeProjectMetadataRaw,
) {
    use std::io::Write as _;

    let info_json = format!(r#"{{"name":"{name}","version":"{version}","usage":[]}}"#);
    // Fixed created-timestamp so the canonical digest is reproducible.
    let meta_json = r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#;

    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file("root/.project.json", options).unwrap();
        zip.write_all(info_json.as_bytes()).unwrap();
        zip.start_file("root/.meta.json", options).unwrap();
        zip.write_all(meta_json.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    let info: sysand_core::model::InterchangeProjectInfoRaw =
        serde_json::from_str(&info_json).expect("hand-written info JSON must parse");
    let meta: sysand_core::model::InterchangeProjectMetadataRaw =
        serde_json::from_str(meta_json).expect("hand-written meta JSON must parse");

    (buf, info, meta)
}

#[test]
fn lock_and_sync_against_mock_index() -> Result<(), Box<dyn std::error::Error>> {
    // End-to-end coverage for the `versions.json`-backed index env:
    //   `sysand init` → inject a `pkg:sysand/<pub>/<name>` usage →
    //   `sysand lock` (reads versions.json, writes lockfile with advertised
    //   project_digest) → `sysand sync` (downloads kpar, verifies kpar_digest,
    //   recomputes canonical digest and reconciles with the lockfile).
    //
    // A regression in which the advertised project_digest cannot actually
    // round-trip through lock+sync (e.g. a non-deterministic fallback hash)
    // would fail the `sync` step with BadChecksum or DigestDrift.
    use sha2::{Digest as _, Sha256};
    use sysand_core::model::project_hash_raw;

    let mut server = Server::new();

    let (kpar_bytes, info, meta) = build_index_kpar_bytes("dep", "0.1.0");
    let kpar_sha256_hex = format!("{:x}", Sha256::digest(&kpar_bytes));
    let project_digest_hex = format!("{:x}", project_hash_raw(&info, &meta));
    let kpar_size = kpar_bytes.len();

    // `sysand lock` targets a specific IRI via `versions_async`, so
    // `index.json` isn't required on this path. Mock it anyway so an
    // accidental enumeration during refactors doesn't surface as a 501.
    let _index_mock = server
        .mock("GET", "/index.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"projects":[{"iri":"pkg:sysand/mock/dep"}]}"#)
        .create();

    let versions_mock = server
        .mock("GET", "/mock/dep/versions.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"versions":[{{"version":"0.1.0","usage":[],"project_digest":"sha256:{project_digest_hex}","kpar_size":{kpar_size},"kpar_digest":"sha256:{kpar_sha256_hex}"}}]}}"#,
        ))
        .expect_at_least(1)
        .create();

    let project_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&info)?)
        .expect_at_least(1)
        .create();

    let meta_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&meta)?)
        .expect_at_least(1)
        .create();

    let kpar_mock = server
        .mock("GET", "/mock/dep/0.1.0/project.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&kpar_bytes)
        .expect_at_least(1)
        .create();

    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--name",
            "lock_and_sync_against_mock_index",
            "--version",
            "1.2.3",
        ],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages(cwd.join(".project.json"), ["pkg:sysand/mock/dep"])?;

    let server_url = server.url();
    let out = run_sysand_in(&cwd, ["lock", "--default-index", &server_url], None)?;
    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;
    assert_eq!(projects.len(), 2, "root + single dep expected");

    let dep = projects
        .iter()
        .find(|p| p.name.as_deref() == Some("dep"))
        .expect("locked dep should carry name from versions.json");
    assert_eq!(
        dep.checksum, project_digest_hex,
        "lockfile must record the advertised canonical digest verbatim"
    );

    let out = run_sysand_in(&cwd, ["sync", "--default-index", &server_url], None)?;
    out.assert().success();

    let entries: Vec<String> =
        std::fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(ENTRIES_PATH))?
            .lines()
            .map(str::to_string)
            .collect();
    assert!(
        entries.contains(&"pkg:sysand/mock/dep".to_string()),
        "env entries should list the synced dep IRI; got {entries:?}"
    );

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();

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
        .stderr(contains("requested version unavailable"));

    Ok(())
}
