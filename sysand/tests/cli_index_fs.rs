// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Filesystem indexes end to end (`design/index-protocol.md` "Filesystem
//! indexes"): a directory `sysand index` wrote and
//! a bare folder of archives, spelled as a path or a `file://` URL, as
//! `--index` and as `--default-index`.
//!
//! Indexes are a union: every configured index is contacted, so "with
//! no network" means the folder is the *only* index (`--default-index`
//! replaces the built-in default). The tests then point every proxy
//! variable at a closed port, which breaks any HTTP index (the control
//! test) and leaves the folder untouched.

use std::error::Error;

use assert_cmd::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use mockito::Server;
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{DEFAULT_ENV_NAME, local_directory::LocalDirectoryEnvironment},
    lock::{Lock, Lockfile, Source},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    purl::PKG_SYSAND_PREFIX,
    utils::sha256_lowercase_hex,
};

mod common;
pub use common::*;

const DEP: &str = "pkg:sysand/mock/dep";

/// Every proxy variable at a closed port: any HTTP request fails.
fn denied_network() -> IndexMap<&'static str, &'static str> {
    IndexMap::from([
        ("HTTP_PROXY", "http://127.0.0.1:9"),
        ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ("ALL_PROXY", "http://127.0.0.1:9"),
        ("http_proxy", "http://127.0.0.1:9"),
        ("https_proxy", "http://127.0.0.1:9"),
        ("NO_PROXY", ""),
        ("no_proxy", ""),
    ])
}

fn add_usage(project_json: &Utf8Path, iri: &str) -> Result<(), Box<dyn Error>> {
    let mut info: InterchangeProjectInfoRaw =
        serde_json::from_str(&std::fs::read_to_string(project_json)?)?;
    info.usage.push(InterchangeProjectUsageRaw::Resource {
        resource: iri.to_owned(),
        version_constraint: None,
    });
    std::fs::write(project_json, serde_json::to_string_pretty(&info)?)?;
    Ok(())
}

/// A project depending on `mock/dep`, and a folder of two archives of it.
fn project_and_folder() -> Result<
    (
        camino_tempfile::Utf8TempDir,
        Utf8PathBuf,
        Utf8PathBuf,
        Vec<u8>,
    ),
    Box<dyn Error>,
> {
    let (temp_dir, cwd) = cli_init_json_project(Some("app"), "acme", "app", "1.2.3", None)?;
    let app = cwd.join("app");
    add_usage(&app.join(".project.json"), DEP)?;
    let folder = cwd.join("mirror");
    std::fs::create_dir_all(&folder)?;
    let (old, _, _) = build_index_kpar_bytes("mock", "dep", "0.1.0");
    std::fs::write(folder.join("dep-0.1.0.kpar"), old)?;
    let (new, _, _) = build_index_kpar_bytes("mock", "dep", "0.2.0");
    std::fs::write(folder.join("Dep-0.2.0.KPAR"), &new)?;
    Ok((temp_dir, app, folder, new))
}

fn read_lock(app: &Utf8Path) -> Result<Lock, Box<dyn Error>> {
    Ok(Lockfile::read(app)?.into_lock())
}

fn locked_dep_source(lock: &Lock) -> Source {
    let dep = lock
        .projects
        .iter()
        .find(|p| p.name == "dep")
        .expect("the dependency is locked");
    assert_eq!(dep.version, "0.2.0", "newest version wins");
    dep.sources[0].clone()
}

#[test]
fn lock_against_folder_of_kpars_network_denied() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, folder, newest) = project_and_folder()?;
    let out = run_sysand_in_with(
        &app,
        ["lock", "--default-index", folder.as_str()],
        None,
        &denied_network(),
    )?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("warning").not());

    let source = locked_dep_source(&read_lock(&app)?);
    match source {
        Source::LocalKpar {
            kpar_path,
            kpar_size,
            kpar_digest,
        } => {
            assert_eq!(kpar_path.as_str(), "../mirror/Dep-0.2.0.KPAR");
            assert_eq!(kpar_size.get(), newest.len() as u64);
            assert_eq!(kpar_digest, sha256_lowercase_hex(&newest));
        }
        other => panic!("a filesystem index locks a local archive, got {other:?}"),
    }

    // The same denied network breaks an HTTP index: the sentinel is real.
    let server = Server::new();
    let out = run_sysand_in_with(
        &app,
        ["lock", "--default-index", &server.url()],
        None,
        &denied_network(),
    )?;
    out.assert().failure();
    Ok(())
}

#[test]
fn lock_and_sync_against_structured_directory_index() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, folder, newest) = project_and_folder()?;
    // Build the structured index with the CLI itself, from the folder's
    // archives.
    let index = app.parent().unwrap().join("index");
    std::fs::create_dir_all(&index)?;
    run_sysand_in(&index, ["index", "init"], None)?
        .assert()
        .success();
    for archive in ["dep-0.1.0.kpar", "Dep-0.2.0.KPAR"] {
        run_sysand_in(
            &index,
            ["index", "add", "--kpar-path", folder.join(archive).as_str()],
            None,
        )?
        .assert()
        .success();
    }
    assert!(index.join("sysand-index-config.json").is_file());

    let out = run_sysand_in_with(
        &app,
        ["lock", "--default-index", index.as_str()],
        None,
        &denied_network(),
    )?;
    out.assert().success();
    match locked_dep_source(&read_lock(&app)?) {
        Source::LocalKpar {
            kpar_path,
            kpar_digest,
            kpar_size: _,
        } => {
            assert_eq!(kpar_path.as_str(), "../index/mock/dep/0.2.0/project.kpar");
            assert_eq!(kpar_digest, sha256_lowercase_hex(&newest));
        }
        other => panic!("a filesystem index locks a local archive, got {other:?}"),
    }

    let out = run_sysand_in_with(
        &app,
        ["sync", "--default-index", index.as_str()],
        None,
        &denied_network(),
    )?;
    out.assert().success();
    let entries: Vec<String> = LocalDirectoryEnvironment::read(app.join(DEFAULT_ENV_NAME))?
        .projects()
        .iter()
        .flat_map(|p| p.identifiers.iter().cloned())
        .collect();
    assert!(
        entries.contains(&format!("{PKG_SYSAND_PREFIX}mock/dep")),
        "env entries should list the synced dep IRI; got {entries:?}"
    );
    Ok(())
}

#[test]
fn file_url_spelling_equals_path_spelling() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, folder, _) = project_and_folder()?;
    run_sysand_in(&app, ["lock", "--default-index", folder.as_str()], None)?
        .assert()
        .success();
    let by_path = std::fs::read_to_string(app.join(DEFAULT_LOCKFILE_NAME))?;
    std::fs::remove_file(app.join(DEFAULT_LOCKFILE_NAME))?;

    let url = file_url_from_path(folder.as_std_path());
    run_sysand_in(&app, ["lock", "--default-index", &url], None)?
        .assert()
        .success();
    let by_url = std::fs::read_to_string(app.join(DEFAULT_LOCKFILE_NAME))?;
    assert_eq!(by_path, by_url);

    // As `--index` next to a default that does not hold the project.
    std::fs::remove_file(app.join(DEFAULT_LOCKFILE_NAME))?;
    let mut server = Server::new();
    let discovery = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(1)
        .create();
    let versions = server
        .mock("GET", "/mock/dep/versions.json")
        .with_status(404)
        .expect(1)
        .create();
    run_sysand_in(
        &app,
        ["lock", "--index", &url, "--default-index", &server.url()],
        None,
    )?
    .assert()
    .success();
    assert_eq!(
        by_path,
        std::fs::read_to_string(app.join(DEFAULT_LOCKFILE_NAME))?
    );
    discovery.assert();
    versions.assert();
    Ok(())
}

#[test]
fn info_iri_answers_from_filesystem_index() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, folder, _) = project_and_folder()?;
    let out = run_sysand_in_with(
        &app,
        ["info", "--iri", DEP, "--default-index", folder.as_str()],
        None,
        &denied_network(),
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: dep"))
        .stdout(predicate::str::contains("Version: 0.2.0"));
    Ok(())
}

#[test]
fn lock_refuses_neither_index_nor_folder() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, _, _) = project_and_folder()?;
    let empty = app.parent().unwrap().join("empty");
    std::fs::create_dir_all(&empty)?;
    let out = run_sysand_in(&app, ["lock", "--default-index", empty.as_str()], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "`{empty}` is neither a Sysand index nor a folder of KPARs"
        )));
    Ok(())
}

#[test]
fn lock_refuses_folder_archive_without_publisher() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, app, folder, _) = project_and_folder()?;
    // A publisher-less archive: the `.project.json` names no publisher.
    let mut buf: Vec<u8> = Vec::new();
    {
        use std::io::Write as _;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"widgets","version":"1.2.3"}"#)?;
        zip.start_file(".meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#)?;
        zip.finish()?;
    }
    let archive = folder.join("widgets.kpar");
    std::fs::write(&archive, buf)?;
    let out = run_sysand_in(&app, ["lock", "--default-index", folder.as_str()], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "`{archive}` has a `.project.json` with no `publisher`"
        )));
    Ok(())
}

#[test]
#[cfg(windows)]
fn windows_spellings_lock_identically() -> Result<(), Box<dyn Error>> {
    // `C:\...` and `file:///C:/...` name the same folder.
    let (_temp_dir, app, folder, _) = project_and_folder()?;
    run_sysand_in(&app, ["lock", "--default-index", folder.as_str()], None)?
        .assert()
        .success();
    let by_path = std::fs::read_to_string(app.join(DEFAULT_LOCKFILE_NAME))?;
    std::fs::remove_file(app.join(DEFAULT_LOCKFILE_NAME))?;
    let url = file_url_from_path(folder.as_std_path());
    assert!(url.starts_with("file:///"), "{url}");
    run_sysand_in(&app, ["lock", "--default-index", &url], None)?
        .assert()
        .success();
    assert_eq!(
        by_path,
        std::fs::read_to_string(app.join(DEFAULT_LOCKFILE_NAME))?
    );
    Ok(())
}
