// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::path::Path;

use assert_cmd::prelude::*;
use mockito::{Mock, Server, ServerGuard};
use predicates::{prelude::*, str::contains};
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{self, ConfigProject, OverrideSource},
    env::{DEFAULT_ENV_NAME, local_directory::LocalDirectoryEnvironment},
    lock::{Lock, Source},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    purl::PKG_SYSAND_PREFIX,
    utils::sha256_lowercase_hex,
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;
use serde_json::json;

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the current working directory. (Non-interactive use)
#[test]
fn lock_trivial() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "lock_trivial", "1.2.3")?;

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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "lock_local_source", "1.2.3")?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = cli_init_project_in(&cwd, Some("local_dep"), "a", None, Some("1.0.0"), None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["add", "urn:kpar:local_dep", "--no-lock"], None)?;

    out.assert().success().stdout(predicate::str::is_empty());

    let cfg = toml::to_string(&config::Config {
        indexes: vec![],
        projects: vec![ConfigProject {
            identifiers: vec!["urn:kpar:local_dep".to_owned()],
            sources: vec![OverrideSource::LocalSrc {
                src_path: "local_dep".into(),
            }],
        }],
    })?;

    let cfg_path = cwd.join(config::local_fs::CONFIG_FILE);
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "lock_std_lib", "1.2.3")?;

    out.assert().success().stdout(predicate::str::is_empty());

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar",
            "--no-lock",
            "--include-std",
        ],
        None,
    )?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: IRI `https://www.omg.org/spec/KerML/20250201/Function-Library.kpar`",
    ));

    let cfg = toml::to_string(&config::Config {
        indexes: vec![],
        projects: vec![sysand_core::config::ConfigProject {
            identifiers: vec!["urn:kpar:local_dep".to_owned()],
            sources: vec![OverrideSource::LocalSrc {
                src_path: cwd.join("local_dep").as_str().into(),
            }],
        }],
    })?;

    let cfg_path = cwd.join(config::local_fs::CONFIG_FILE);
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
    mocks: &mut Vec<Mock>,
    expected_counts: [usize; 4],
    path: P,
    name: N,
    version: V,
    deps: I,
) -> String {
    let usage: Vec<serde_json::Value> = deps
        .into_iter()
        .map(|dep| json!({"resource": dep.as_ref()}))
        .collect();

    let path = path.as_ref();
    let project_body =
        json!({"name": name.as_ref(), "version": version.as_ref(), "usage": usage}).to_string();
    let meta_body = json!({"index":{}, "created": "0000-00-00T00:00:00.123456789Z"}).to_string();
    let [project_head, project_get, meta_head, meta_get] = expected_counts;

    mocks.push(
        server
            .mock("HEAD", format!("/{path}/.project.json").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&project_body)
            .expect(project_head)
            .create(),
    );

    mocks.push(
        server
            .mock("GET", format!("/{path}/.project.json").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(project_body)
            .expect(project_get)
            .create(),
    );

    mocks.push(
        server
            .mock("HEAD", format!("/{path}/.meta.json").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&meta_body)
            .expect(meta_head)
            .create(),
    );

    mocks.push(
        server
            .mock("GET", format!("/{path}/.meta.json").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(meta_body)
            .expect(meta_get)
            .create(),
    );

    format!("{}/{}", server.url(), path)
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
        info.usage.push(InterchangeProjectUsageRaw::Resource {
            resource: usage.as_ref().to_owned(),
            version_constraint: version_req.map(|x| x.as_ref().to_owned()),
        });
    }

    std::fs::write(&project_path, serde_json::to_string(&info)?)?;

    Ok(())
}

#[test]
fn lock_basic_http_deps() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::new();
    let mut project_mocks = Vec::new();

    let c_url = mock_project(
        &mut server,
        &mut project_mocks,
        [1, 8, 1, 6],
        "c",
        "lock_basic_http_deps_c",
        "1.0.0",
        NO_DEP,
    );

    let a_url = mock_project(
        &mut server,
        &mut project_mocks,
        [1, 8, 1, 6],
        "a",
        "lock_basic_http_deps_a",
        "1.0.0",
        [&c_url],
    );
    let b_url = mock_project(
        &mut server,
        &mut project_mocks,
        [1, 8, 1, 6],
        "b",
        "lock_basic_http_deps_b",
        "1.0.0",
        [&c_url],
    );

    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "lock_basic_http_deps", "1.2.3")?;

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

    assert!(project_names.contains(&"lock_basic_http_deps".to_owned()));
    assert!(project_names.contains(&"lock_basic_http_deps_a".to_owned()));
    assert!(project_names.contains(&"lock_basic_http_deps_b".to_owned()));
    assert!(project_names.contains(&"lock_basic_http_deps_c".to_owned()));

    run_sysand_in(&cwd, ["env"], None)?
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["sync"], None)?;
    out.assert().success().stdout(predicate::str::is_empty());

    let entries: Vec<String> = LocalDirectoryEnvironment::read(cwd.join(DEFAULT_ENV_NAME))?
        .projects()
        .iter()
        .flat_map(|p| p.identifiers.iter().cloned())
        .collect();

    assert_eq!(entries.len(), 3);

    assert!(entries.contains(&a_url));
    assert!(entries.contains(&b_url));
    assert!(entries.contains(&c_url));

    for mock in project_mocks {
        mock.assert();
    }

    Ok(())
}

/// Build a minimal valid kpar with `.project.json` and `.meta.json`.
///
/// The fixture has no `meta.checksum` entries, so its canonical project
/// digest is `project_hash_raw(info, meta)`.
fn build_index_kpar_bytes(
    name: &str,
    version: &str,
) -> (
    Vec<u8>,
    sysand_core::model::InterchangeProjectInfoRaw,
    sysand_core::model::InterchangeProjectMetadataRaw,
) {
    use std::io::Write as _;

    let info_json = format!(r#"{{"name":"{name}","version":"{version}"}}"#);
    // Fixed created-timestamp so the digest is reproducible.
    let meta_json = r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#;

    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file(".project.json", options).unwrap();
        zip.write_all(info_json.as_bytes()).unwrap();
        zip.start_file(".meta.json", options).unwrap();
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
    // End-to-end check that an index-advertised project digest round-trips
    // through lockfile writing and sync-time archive verification.

    let mut server = Server::new();

    let (kpar_bytes, info, meta) = build_index_kpar_bytes("dep", "0.1.0");
    let kpar_sha256_hex = sha256_lowercase_hex(&kpar_bytes);
    // No `meta.checksum` entries → canonical digest == raw digest for this
    // fixture; see the docstring on `build_index_kpar_bytes`.
    let kpar_size = kpar_bytes.len();

    // `sysand lock` targets a specific IRI via `versions_async`; it must not
    // enumerate `index.json`, which is a comparatively expensive operation.
    let index_mock = server
        .mock("GET", "/index.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            r#"{{"projects":[{{"iri":"{PKG_SYSAND_PREFIX}mock/dep"}}]}}"#
        ))
        .expect(0)
        .create();

    // Discovery: no document present means `index_root` defaults to the
    // discovery root (reads need only `index_root`; `api_root` stays
    // unset).
    let config_mock = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(1)
        .create();

    let versions_mock = server
        .mock("GET", "/mock/dep/versions.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(versions_json_body(&[versions_json_entry_body(
            "0.1.0",
            kpar_size,
            &kpar_sha256_hex,
        )]))
        .expect(1)
        .create();

    let project_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&info)?)
        .expect(1)
        .create();

    let meta_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&meta)?)
        .expect(1)
        .create();

    let kpar_mock = server
        .mock("GET", "/mock/dep/0.1.0/project.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&kpar_bytes)
        .expect(1)
        .create();

    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "lock_and_sync_against_mock_index", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages(
        cwd.join(".project.json"),
        [format!("{PKG_SYSAND_PREFIX}mock/dep")],
    )?;

    let server_url = server.url();
    let out = run_sysand_in(&cwd, ["lock", "--default-index", &server_url], None)?;
    out.assert().success().stdout(predicate::str::is_empty());

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;
    assert_eq!(projects.len(), 2, "root + single dep expected");

    let dep = projects
        .iter()
        .find(|p| p.name == "dep")
        .expect("locked dep should carry name from versions.json");
    assert!(
        dep.sources.iter().any(|source| {
            matches!(
                source,
                Source::IndexKpar {
                    kpar_digest,
                    ..
                } if kpar_digest == &kpar_sha256_hex
            )
        }),
        "lockfile must retain the advertised kpar_digest for sync-time verification"
    );

    let out = run_sysand_in(&cwd, ["sync", "--default-index", &server_url], None)?;
    out.assert().success();

    let entries: Vec<String> = LocalDirectoryEnvironment::read(cwd.join(DEFAULT_ENV_NAME))?
        .projects()
        .iter()
        .flat_map(|p| p.identifiers.iter().cloned())
        .collect();
    assert!(
        entries.contains(&format!("{PKG_SYSAND_PREFIX}mock/dep")),
        "env entries should list the synced dep IRI; got {entries:?}"
    );

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();
    config_mock.assert();
    index_mock.assert();

    Ok(())
}

#[test]
fn sync_hard_fails_on_kpar_digest_drift_from_lockfile() -> Result<(), Box<dyn std::error::Error>> {
    // Server immutability + lockfile digest tripwire: `lock` records both the
    // canonical project digest and the raw archive digest. A later `sync` must
    // reject a different archive at the same URL even if the URL itself is
    // unchanged.

    let mut server = Server::new();

    // Build the lock-time view: the digests below are what end up in the
    // lockfile.
    let (kpar_bytes, info, meta) = build_index_kpar_bytes("dep", "0.1.0");
    let kpar_digest_hex = sha256_lowercase_hex(&kpar_bytes);
    let kpar_size = kpar_bytes.len();

    let config_mock = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(1)
        .create();

    let versions_mock = server
        .mock("GET", "/mock/dep/versions.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(versions_json_body(&[versions_json_entry_body(
            "0.1.0",
            kpar_size,
            &kpar_digest_hex,
        )]))
        .expect(1)
        .create();

    let project_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&info)?)
        .expect(1)
        .create();

    let meta_json_mock = server
        .mock("GET", "/mock/dep/0.1.0/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&meta)?)
        .expect(1)
        .create();

    let kpar_mock = server
        .mock("GET", "/mock/dep/0.1.0/project.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&kpar_bytes)
        .expect(0)
        .create();

    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "sync_tripwire", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages(
        cwd.join(".project.json"),
        [format!("{PKG_SYSAND_PREFIX}mock/dep")],
    )?;

    let server_url = server.url();
    let out = run_sysand_in(&cwd, ["lock", "--default-index", &server_url], None)?;
    out.assert().success().stdout(predicate::str::is_empty());

    // Sanity-check: lockfile recorded the advertised kpar_digest.
    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let dep = lock_file
        .projects
        .iter()
        .find(|p| p.name == "dep")
        .expect("locked dep present");
    assert!(
        dep.sources.iter().any(|source| {
            matches!(
                source,
                Source::IndexKpar {
                    kpar_digest,
                    ..
                } if kpar_digest == &kpar_digest_hex
            )
        }),
        "lockfile must retain the advertised kpar_digest for sync-time verification"
    );

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();
    config_mock.assert();

    // Drift the server: keep the URL stable but swap the archive bytes. `sync`
    // should now fail on the recorded archive digest before installation.
    server.reset();
    let (drifted_kpar_bytes, _drifted_info, _drifted_meta) = build_index_kpar_bytes("dep", "0.1.0");
    let mut drifted_kpar_bytes = drifted_kpar_bytes;
    let first_byte = drifted_kpar_bytes
        .first_mut()
        .expect("test kpar builder should produce non-empty archive bytes");
    *first_byte ^= 0xff;
    let drifted_kpar_mock = server
        .mock("GET", "/mock/dep/0.1.0/project.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&drifted_kpar_bytes)
        .expect(1)
        .create();

    // Now sync. The stored `kpar_digest` should reject the drifted bytes
    // before any install happens.
    let out = run_sysand_in(&cwd, ["sync", "--default-index", &server_url], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("sha256"))
        .stderr(predicate::str::contains("expected digest"));

    drifted_kpar_mock.assert();

    Ok(())
}

#[test]
fn lock_rejects_non_normalized_sysand_purl() -> Result<(), Box<dyn std::error::Error>> {
    // A `pkg:sysand/<publisher>/<name>` IRI in a project's declared usages
    // that isn't normalized (uppercase, spaces, wrong segment count, etc.)
    // must be rejected at validation time with an error that names the
    // offending IRI and shows the suggested normalized form, rather than
    // being silently rerouted to `_iri/<sha256>/` and surfacing only as
    // an opaque "not found" downstream.
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "lock_rejects_non_normalized_sysand_purl", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages(
        cwd.join(".project.json"),
        [format!("{PKG_SYSAND_PREFIX}Acme Labs/My.Project")],
    )?;

    let out = run_sysand_in(&cwd, ["lock"], None)?;
    out.assert()
        .failure()
        .stderr(contains(format!("{PKG_SYSAND_PREFIX}Acme Labs/My.Project")))
        .stderr(contains(format!("{PKG_SYSAND_PREFIX}acme-labs/my.project")));

    Ok(())
}

/// A transitive directory usage must be resolved relative to the project
/// that declares it, not relative to the top-level project being locked:
/// `app` uses `deps/widget`, and `widget` uses `../gadget`, which points at
/// `deps/gadget` only when resolved against `widget`'s own directory. If the
/// solver kept passing the top-level base path down, `../gadget` would land
/// outside the temporary directory and resolution would fail
#[test]
fn lock_directory_usage_transitive() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "app", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    let widget_dir = cwd.join("deps").join("widget");
    std::fs::create_dir_all(&widget_dir)?;
    cli_init_project_in(&widget_dir, None, "b", Some("widget"), Some("1.0.0"), None)?
        .assert()
        .success();

    let gadget_dir = cwd.join("deps").join("gadget");
    std::fs::create_dir_all(&gadget_dir)?;
    cli_init_project_in(&gadget_dir, None, "c", Some("gadget"), Some("2.0.0"), None)?
        .assert()
        .success();

    run_sysand_in(
        &cwd,
        ["experimental", "add", "--no-lock", "--dir", "deps/widget"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &widget_dir,
        ["experimental", "add", "--no-lock", "--dir", "../gadget"],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(&cwd, ["lock"], None)?;
    out.assert().success();

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(
        projects.len(),
        3,
        "expected app, widget and gadget in the lockfile, got: {projects:#?}"
    );

    // Resolved paths are relativized back against the top-level project
    // root when recorded in the lockfile
    for (name, version, expected_src_path) in [
        ("widget", "1.0.0", "deps/widget"),
        ("gadget", "2.0.0", "deps/gadget"),
    ] {
        let project = projects
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("`{name}` missing from lockfile: {projects:#?}"));
        assert_eq!(project.version, version);
        let [Source::LocalSrc { src_path, .. }] = project.sources.as_slice() else {
            panic!("expected a single local source for `{name}`, got: {project:#?}");
        };
        assert_eq!(src_path.as_str(), expected_src_path);
    }

    Ok(())
}

/// A dependency resolved *from the environment* can declare a directory
/// usage whose relative path only made sense in its original source tree:
/// `app` was authored next to `widget` and uses `../widget`, but once `app`
/// is installed into `.sysand`, that path (relative to the env-internal
/// project root) points at nothing. Resolution must then fall back to the
/// environment, where the usage's publisher/name identify `widget`, instead
/// of failing the whole lock on the file resolver's `NotFound`
#[test]
fn lock_directory_usage_env_installed_dependency() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "consumer", "1.0.0")?;
    out.assert().success();

    // Source tree: `app` uses `widget` via the directory usage `../widget`
    let app_dir = cwd.join("src").join("app");
    let widget_dir = cwd.join("src").join("widget");
    std::fs::create_dir_all(&app_dir)?;
    std::fs::create_dir_all(&widget_dir)?;
    cli_init_project_in(
        &widget_dir,
        None,
        "acme",
        Some("widget"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();
    cli_init_project_in(&app_dir, None, "acme", Some("app"), Some("1.0.0"), None)?
        .assert()
        .success();
    run_sysand_in(
        &app_dir,
        ["experimental", "add", "--no-lock", "--dir", "../widget"],
        None,
    )?
    .assert()
    .success();

    // Install both projects into the consumer's environment, then delete the
    // source tree: the env copies are the only ones left, and the `../widget`
    // recorded in `app` now points at a non-existent path inside `.sysand`
    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "pkg:sysand/acme/widget",
            "--path",
            "src/widget",
            "--no-deps",
            "--no-index",
        ],
        None,
    )?
    .assert()
    .success();
    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "pkg:sysand/acme/app",
            "--path",
            "src/app",
            "--no-deps",
            "--no-index",
        ],
        None,
    )?
    .assert()
    .success();
    std::fs::remove_dir_all(cwd.join("src"))?;

    run_sysand_in(&cwd, ["add", "acme/app", "--no-lock", "--no-index"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["lock", "--no-index"], None)?;
    out.assert().success();

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let projects = lock_file.projects;

    assert_eq!(
        projects.len(),
        3,
        "expected consumer, app and widget in the lockfile, got: {projects:#?}"
    );
    for name in ["app", "widget"] {
        let project = projects
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("`{name}` missing from lockfile: {projects:#?}"));
        assert_eq!(project.version, "1.0.0");
    }

    Ok(())
}

#[test]
fn lock_fail_unsatisfiable() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = Server::new();
    let mut project_mocks = Vec::new();

    let a_url = mock_project(
        &mut server,
        &mut project_mocks,
        [1, 4, 1, 2],
        "a",
        "lock_basic_http_deps_a",
        "1.0.0",
        NO_DEP,
    );

    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "lock_basic_http_deps", "1.2.3")?;

    out.assert().success().stdout(predicate::str::is_empty());

    inject_usages_versions(cwd.join(".project.json"), [(a_url, Some(">1.0.0"))])?;

    let out = run_sysand_in(&cwd, ["lock"], None)?;

    out.assert()
        .failure()
        .stderr(contains("requested version unavailable"));

    for mock in project_mocks {
        mock.assert();
    }

    Ok(())
}
