// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Tests for the filesystem index in both shapes (`design/index-protocol.md`
//! "Filesystem indexes").

use std::io::Write as _;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use futures::StreamExt as _;

use super::*;
use crate::{
    commands::index::{do_index_add, do_index_init},
    context::ProjectContext,
    lock::Source,
    project::ProjectReadAsync as _,
    utils::sha256_lowercase_hex,
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
}

/// A minimal archive: `.project.json` and `.meta.json` at the root.
fn write_kpar(path: &Utf8Path, publisher: Option<&str>, name: &str, version: &str) -> Vec<u8> {
    let info = match publisher {
        Some(p) => format!(r#"{{"name":"{name}","publisher":"{p}","version":"{version}"}}"#),
        None => format!(r#"{{"name":"{name}","version":"{version}"}}"#),
    };
    let meta = r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file(".project.json", options).unwrap();
        zip.write_all(info.as_bytes()).unwrap();
        zip.start_file(".meta.json", options).unwrap();
        zip.write_all(meta.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    std::fs::write(path, &buf).unwrap();
    buf
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
    ProjectContext {
        model_workspace_root: Some(root.to_path_buf()),
        ..ProjectContext::default()
    }
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
    let (root, bytes) = structured_index(cwd.path());
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);

    assert_eq!(uris(&rt, &env), vec![DEP.to_owned()]);
    assert_eq!(versions(&rt, &env, DEP), vec!["0.2.0", "0.1.0"]);

    let project = rt.block_on(env.get_project_async(DEP, "0.2.0")).unwrap();
    let (info, meta) = rt.block_on(project.get_project_async()).unwrap();
    let info = info.expect("info");
    assert_eq!(info.name, "dep");
    assert_eq!(info.publisher.as_deref(), Some("mock"));
    assert_eq!(info.version, "0.2.0");
    assert!(meta.is_some());

    // The lockfile source is the archive relative to the workspace root,
    // carrying the entry's size and digest (verified on open).
    let ctx = workspace_ctx(cwd.path());
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
        [] as [std::string::String; 0]
    );
    let error = rt
        .block_on(env.get_project_async("pkg:sysand/mock/other", "1.0.0"))
        .unwrap_err();
    assert_matches!(error, LocalIndexError::ProjectNotInIndex { .. });
    let error = rt
        .block_on(env.get_project_async(DEP, "9.9.9"))
        .unwrap_err();
    assert_matches!(error, LocalIndexError::VersionNotInIndex { .. });
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
    assert_matches!(error, LocalKParError::SizeMismatch { .. });
}

#[test]
fn folder_index_enumerates_case_insensitively() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    write_kpar(&root.join("a.kpar"), Some("mock"), "dep", "0.1.0");
    write_kpar(&root.join("B.KPAR"), Some("mock"), "dep", "0.2.0");
    write_kpar(&root.join("c.Kpar"), Some("mock"), "other", "1.0.0");
    std::fs::write(root.join("notes.txt"), "not an archive").unwrap();
    // Not recursive: a nested archive is not part of the folder.
    std::fs::create_dir_all(root.join("sub")).unwrap();
    write_kpar(&root.join("sub/d.kpar"), Some("mock"), "nested", "1.0.0");

    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    assert_eq!(
        uris(&rt, &env),
        vec![DEP.to_owned(), "pkg:sysand/mock/other".to_owned()]
    );
    assert_eq!(versions(&rt, &env, DEP), vec!["0.2.0", "0.1.0"]);
    assert_eq!(
        versions(&rt, &env, "pkg:sysand/mock/nested"),
        [] as [std::string::String; 0]
    );

    let project = rt.block_on(env.get_project_async(DEP, "0.2.0")).unwrap();
    let info = rt.block_on(project.get_info_async()).unwrap().unwrap();
    assert_eq!(info.version, "0.2.0");
    let ctx = workspace_ctx(cwd.path());
    let sources = rt.block_on(project.sources_async(&ctx)).unwrap();
    assert_matches!(
        sources.as_slice(),
        [Source::LocalKpar { kpar_path, .. }] if kpar_path.as_str() == "folder/B.KPAR"
    );
}

#[test]
fn folder_archive_without_publisher_is_refused() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    write_kpar(&root.join("widgets.kpar"), None, "widgets", "1.2.3");

    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let error = rt.block_on(env.versions_async(DEP)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "`{archive}` has a `.project.json` with no `publisher`;\n\
            {SP:>8} a folder index names each archive `pkg:sysand/<publisher>/<name>`, \
            so add one and rebuild the archive",
            archive = root.join("widgets.kpar"),
            SP = ' '
        )
    );
}

#[test]
fn folder_non_semver_version_is_refused() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    write_kpar(&root.join("a.kpar"), Some("mock"), "dep", "latest");
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let error = rt.block_on(env.uris_async()).unwrap_err();
    assert_matches!(error, LocalIndexError::NonSemverArchiveVersion { value, .. } if value == "latest");
}

#[test]
fn folder_duplicate_identity_version_is_refused() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    write_kpar(&root.join("a.kpar"), Some("mock"), "dep", "0.1.0");
    write_kpar(&root.join("b.kpar"), Some("mock"), "dep", "0.1.0");
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let error = rt.block_on(env.uris_async()).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "`{a}` and `{b}` in `{root}` both hold `pkg:sysand/mock/dep` `0.1.0`",
            a = root.join("a.kpar"),
            b = root.join("b.kpar"),
        )
    );
}

#[test]
fn neither_index_nor_folder_is_refused() {
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
            "`{root}` is neither a Sysand index nor a folder of KPARs"
        )),
        "{text}"
    );

    // A missing directory is its own message, not `NeitherIndexNorFolder`.
    let env = LocalIndexEnvironment::new(cwd.path().join("absent"));
    let error = rt.block_on(env.versions_async(DEP)).unwrap_err();
    assert_matches!(error, LocalIndexError::NotADirectory { .. });
}

#[test]
#[cfg(unix)]
fn symlinked_archive_outside_root_is_refused() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    let outside = cwd.path().join("outside.kpar");
    write_kpar(&outside, Some("mock"), "dep", "0.1.0");
    std::os::unix::fs::symlink(&outside, root.join("link.kpar")).unwrap();

    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let error = rt.block_on(env.versions_async(DEP)).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "`{entry}` in `{root}` escapes the index at `{root}`",
            entry = root.join("link.kpar"),
        )
    );
}

#[test]
#[cfg(unix)]
fn versions_entry_escaping_root_is_refused() {
    let cwd = tempdir().unwrap();
    let (root, _) = structured_index(cwd.path());
    // Replace the 0.2.0 version directory with a symlink out of the index.
    let outside = cwd.path().join("elsewhere");
    std::fs::rename(root.join("mock/dep/0.2.0"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("mock/dep/0.2.0")).unwrap();

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

#[test]
fn folder_enumeration_cost_100_archives() {
    let cwd = tempdir().unwrap();
    let root = cwd.path().join("folder");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..100 {
        write_kpar(
            &root.join(format!("dep{i:03}.kpar")),
            Some("mock"),
            &format!("dep{i:03}"),
            "1.0.0",
        );
    }
    let rt = runtime();
    let env = LocalIndexEnvironment::new(&root);
    let started = std::time::Instant::now();
    assert_eq!(uris(&rt, &env).len(), 100);
    let first_pass = started.elapsed();
    // The table is built once per environment lifetime.
    let started = std::time::Instant::now();
    assert_eq!(versions(&rt, &env, "pkg:sysand/mock/dep042"), vec!["1.0.0"]);
    let lookup = started.elapsed();
    eprintln!(
        "folder enumeration of 100 archives: {} ms; cached lookup: {} us",
        first_pass.as_millis(),
        lookup.as_micros()
    );
}

/// A `log::Log` that keeps warning texts, for the no-warning rule
/// (neither shape warns about the other). Tests run
/// in parallel, so the assertion filters by the test's own paths rather
/// than requiring silence from the whole binary.
struct WarnCapture(std::sync::Mutex<Vec<String>>);

impl log::Log for WarnCapture {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if record.level() <= log::Level::Warn {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static WARNINGS: WarnCapture = WarnCapture(std::sync::Mutex::new(Vec::new()));

#[test]
fn no_warning_in_either_shape() {
    // Another test in this binary may already have installed a logger;
    // then this one cannot observe and passes vacuously.
    if log::set_logger(&WARNINGS).is_err() {
        return;
    }
    log::set_max_level(log::LevelFilter::Warn);
    let cwd = tempdir().unwrap();
    let (structured, _) = structured_index(cwd.path());
    let folder = cwd.path().join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    write_kpar(&folder.join("a.kpar"), Some("mock"), "dep", "0.1.0");

    let rt = runtime();
    for root in [structured, folder] {
        let env = LocalIndexEnvironment::new(&root);
        assert_eq!(
            versions(&rt, &env, DEP).len(),
            if root.ends_with("folder") { 1 } else { 2 }
        );
        rt.block_on(env.get_project_async(DEP, "0.1.0")).unwrap();
    }
    let mine: Vec<String> = WARNINGS
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|text| text.contains(cwd.path().as_str()))
        .cloned()
        .collect();
    assert!(mine.is_empty(), "warnings about the index: {mine:?}");
}

use std::assert_matches;
