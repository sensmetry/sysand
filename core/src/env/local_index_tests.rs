// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Tests for the filesystem index (`design/index-protocol.md` "Filesystem
//! indexes").

use camino_tempfile::tempdir;
use futures::StreamExt as _;
#[cfg(unix)]
use std::os::unix::fs::symlink;

use super::*;
use crate::{
    commands::index::{do_index_add, do_index_init},
    context::ProjectContext,
    lock::Source,
    project::{ProjectDefinition, ProjectReadAsync as _, local_kpar::test_support::write_kpar},
    utils::sha256_lowercase_hex,
    workspace::model_toml,
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
}

fn versions(rt: &tokio::runtime::Runtime, env: &LocalIndexEnvironment, iri: &str) -> Vec<String> {
    rt.block_on(async {
        env.versions_async(iri)
            .await
            .expect("versions stream")
            .map(|v| v.expect("version"))
            .collect::<Vec<_>>()
            .await
    })
}

fn uris(rt: &tokio::runtime::Runtime, env: &LocalIndexEnvironment) -> Vec<String> {
    rt.block_on(async {
        env.uris_async()
            .await
            .expect("uris stream")
            .map(|v| v.expect("uri"))
            .collect::<Vec<_>>()
            .await
    })
}

const DEP: &str = "pkg:sysand/mock/dep";

/// A context whose lockfile lives at `root`, so `sources` are relative
/// to it.
fn workspace_ctx(root: &Utf8Path) -> ProjectContext {
    std::fs::write(
        root.join("model.toml"),
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = []\n",
    )
    .unwrap();
    ProjectContext::new(Some(model_toml::Workspace::load(root).unwrap()), None, None)
}

/// A structured index written by the real `index init` + `index add`,
/// holding `mock/dep` 0.1.0 and 0.2.0; returns (root, bytes of 0.2.0).
fn structured_index(cwd: &Utf8Path) -> (Utf8PathBuf, Vec<u8>) {
    let root = cwd.join("index");
    do_index_init(&root).unwrap();
    let a = cwd.join("dep-0.1.0.kpar");
    write_kpar(&a, Some("mock"), "dep", "0.1.0");
    let b = cwd.join("dep-0.2.0.kpar");
    let bytes = write_kpar(&b, Some("mock"), "dep", "0.2.0");
    do_index_add::<&str, _, _>(None, &a, &root).unwrap();
    do_index_add::<&str, _, _>(None, &b, &root).unwrap();
    (root, bytes)
}

#[test]
fn structured_index_lists_versions_and_reads_archive() {
    let cwd = tempdir().unwrap();
    let cwd = wrapfs::canonicalize(cwd.path()).unwrap();
    let (root, bytes) = structured_index(&cwd);
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);

    assert_eq!(uris(&rt, &env), vec![DEP.to_owned()]);
    assert_eq!(versions(&rt, &env, DEP), vec!["0.2.0", "0.1.0"]);

    let project = rt.block_on(env.get_project_async(DEP, "0.2.0")).unwrap();
    let (info, meta) = rt
        .block_on(project.get_project_async())
        .unwrap()
        .and_then(ProjectDefinition::into_json)
        .unzip();
    let info = info.expect("info");
    assert_eq!(info.name, "dep");
    assert_eq!(info.publisher.as_deref(), Some("mock"));
    assert_eq!(info.version, "0.2.0");
    assert!(meta.is_some());

    // The lockfile source is the archive relative to the workspace root,
    // carrying the entry's size and digest (verified on open).
    let ctx = workspace_ctx(&cwd);
    let sources = rt.block_on(project.sources_async(&ctx)).unwrap();
    assert_eq!(sources.len(), 1);
    match &sources[0] {
        Source::LocalKpar {
            kpar_path,
            kpar_size,
            kpar_digest,
        } => {
            assert_eq!(kpar_path.as_str(), "index/mock/dep/0.2.0/project.kpar");
            assert_eq!(kpar_size.get(), bytes.len() as u64);
            assert_eq!(kpar_digest, &sha256_lowercase_hex(&bytes));
        }
        other => panic!("expected a local kpar source, got {other:?}"),
    }
}

#[test]
fn structured_index_missing_versions_is_empty_stream() {
    let cwd = tempdir().unwrap();
    let (root, _) = structured_index(cwd.path());
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    assert_eq!(
        versions(&rt, &env, "pkg:sysand/mock/other"),
        Vec::<String>::new()
    );
    let error = rt
        .block_on(env.get_project_async("pkg:sysand/mock/other", "1.0.0"))
        .unwrap_err();
    assert_matches!(
        error,
        LocalIndexError::ProjectNotInIndex { file: _, iri: _ }
    );
    let error = rt
        .block_on(env.get_project_async(DEP, "9.9.9"))
        .unwrap_err();
    assert_matches!(
        error,
        LocalIndexError::VersionNotInIndex {
            file: _,
            version: _
        }
    );
}

#[test]
fn structured_index_digest_mismatch_is_refused() {
    let cwd = tempdir().unwrap();
    let (root, _) = structured_index(cwd.path());
    // Tamper with the archive after the entry was written.
    let archive = root.join("mock/dep/0.2.0/project.kpar");
    let mut bytes = std::fs::read(&archive).unwrap();
    bytes.push(0);
    std::fs::write(&archive, bytes).unwrap();

    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let project = rt.block_on(env.get_project_async(DEP, "0.2.0")).unwrap();
    let error = rt.block_on(project.get_project_async()).unwrap_err();
    assert_matches!(
        error,
        LocalKParError::SizeMismatch {
            path: _,
            expected: _,
            actual: _
        }
    );
}

#[test]
fn directory_without_index_json_is_refused() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("empty");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("README"), "nothing here").unwrap();
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let error = rt.block_on(env.versions_async(DEP)).unwrap_err();
    let text = error.to_string();
    assert!(
        text.starts_with(&format!(
            "`{root}` is not a Sysand index: it has no `index.json`"
        )),
        "{text}"
    );

    // A missing directory is its own message, not `NotAnIndex`.
    let env = LocalIndexEnvironment::new(cwd.path().join("absent"));
    let error = rt.block_on(env.versions_async(DEP)).unwrap_err();
    assert_matches!(error, LocalIndexError::NotADirectory { path: _ });
}

#[test]
#[cfg(unix)]
fn versions_entry_escaping_root_is_refused() {
    let cwd = tempdir().unwrap();
    let (root, _) = structured_index(cwd.path());
    // Replace the 0.2.0 version directory with a symlink out of the index.
    let outside = cwd.path().join("elsewhere");
    std::fs::rename(root.join("mock/dep/0.2.0"), &outside).unwrap();
    symlink(&outside, root.join("mock/dep/0.2.0")).unwrap();

    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    // Enumeration still works: the entry is named by versions.json.
    assert_eq!(versions(&rt, &env, DEP), vec!["0.2.0", "0.1.0"]);
    let error = rt
        .block_on(env.get_project_async(DEP, "0.2.0"))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "`{entry}` in `{file}` escapes the index at `{root}`",
            entry = root.join("mock/dep/0.2.0/project.kpar"),
            file = root.join("mock/dep/versions.json"),
        )
    );
    // The untouched version is still served.
    rt.block_on(env.get_project_async(DEP, "0.1.0")).unwrap();
}

use std::assert_matches;
