// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![allow(unused_imports)]

use std::io::Read as _;

#[cfg(feature = "alltests")]
use camino::Utf8Path;
use camino_tempfile::tempdir;

use crate::{
    context::ProjectContext,
    lock::Source,
    project::{ProjectRead as _, gix_git_download::GixDownloadedProject},
};
//use predicates::prelude::*;

/// Writes `files` (relative path, content) into `dir`, makes `dir` a git
/// repository and commits them, leaving an index that matches the commit.
///
/// Built with gix in isolation rather than with the `git` binary: no
/// system, user or environment git configuration is read, so nothing on
/// the machine running the tests (commit signing, hooks, a default branch)
/// changes the repository.
#[cfg(feature = "alltests")]
fn git_commit_files(
    dir: &Utf8Path,
    files: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::ThreadSafeRepository::init_opts(
        dir,
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )?
    .to_thread_local();
    let mut tree = repo.edit_tree(gix::ObjectId::empty_tree(repo.object_hash()))?;
    for (path, content) in files {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, content)?;
        tree.upsert(
            *path,
            gix::object::tree::EntryKind::Blob,
            repo.write_blob(content)?,
        )?;
    }
    let tree = tree.write()?.detach();
    let signature = gix::actor::SignatureRef {
        name: "Test User".into(),
        email: "user@sysand.com".into(),
        time: "0 +0000",
    };
    repo.commit_as(
        signature,
        signature,
        "HEAD",
        "test_commit",
        tree,
        gix::commit::NO_PARENT_IDS,
    )?;
    repo.index_from_tree(&tree)?
        .write(gix::index::write::Options::default())?;
    Ok(())
}

/// Not isolated from the machine's git configuration: the clone runs gix
/// in the test process, and the `git upload-pack` it starts for `file://`
/// inherits the test's environment, so both read the system and user git
/// config. Isolating them needs a cleared environment, which is only sound
/// before any thread exists; as an integration test that means its own
/// `main` (`harness = false`, a `[[test]]` entry, `unsafe` environment
/// edits), which is too much special-casing for one test.
#[cfg(feature = "alltests")]
#[test]
pub fn basic_gix_access() -> Result<(), Box<dyn std::error::Error>> {
    use crate::project::utils::wrapfs;

    let repo_dir = tempdir()?;
    git_commit_files(
        repo_dir.path(),
        &[
            (
                ".project.json",
                r#"{"name":"basic_gix_access","version":"1.2.3"}"#,
            ),
            (".meta.json", r#"{"index":{},"created":"123"}"#),
            ("test.sysml", "package Test;"),
        ],
    )?;

    // NOTE: Gix does not support the "dumb" HTTP protocol

    // let free_port = port_check::free_local_port().unwrap().to_string();
    // let mut server = Command::new("uv")
    //     .arg("run")
    //     .arg("--isolated")
    //     .arg("--with")
    //     .arg("rangehttpserver")
    //     .arg("-m")
    //     .arg("RangeHTTPServer")
    //     .arg(&free_port)
    //     .current_dir(repo_dir.path().join(".git"))
    //     .spawn()?;

    // sleep(Duration::from_millis(100));

    let path = wrapfs::canonicalize(repo_dir.path())?;
    let project = GixDownloadedProject::new(format!("file://{path}"))?;

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!("expected info and meta");
    };

    assert_eq!(info.name, "basic_gix_access");
    assert_eq!(meta.created, "123");

    let mut buf = String::new();
    project
        .read_source("test.sysml")?
        .read_to_string(&mut buf)?;
    assert_eq!(buf, "package Test;");

    // server.kill()?;
    Ok(())
}

/// `gix::Url` serializes an ssh URL with an IPv6 host without the
/// brackets (`ssh://::1/r.git`), which is not a valid IRI, so the
/// `Iri::parse(..).unwrap()` in `sources` panics.
#[test]
#[should_panic(expected = "called `Result::unwrap()` on an `Err` value")]
fn sources_panics_on_ssh_ipv6_host() {
    let project =
        GixDownloadedProject::new("ssh://[::1]/r.git").expect("gix accepts an IPv6 ssh URL");
    let _sources = project.sources(&ProjectContext::default());
}

/// The `remote_git` lockfile source recorded for `url`
fn remote_git_source(url: &str) -> String {
    let project = GixDownloadedProject::new(url).expect("gix accepts the URL");
    match project
        .sources(&ProjectContext::default())
        .expect("sources needs no download")
        .as_slice()
    {
        [Source::RemoteGit { remote_git }] => remote_git.to_string(),
        other => panic!("expected a single remote git source, got {other:?}"),
    }
}

/// NOT INTENDED (known bug): `gix::Url`'s `Display` replaces the password
/// with `redacted`, and `sources` records that, so the lockfile source
/// differs from the URL the user wrote and a later sync from the lockfile
/// authenticates with the literal password `redacted`. The source should
/// be the URL as written; update these assertions when that is fixed.
#[test]
fn sources_record_a_redacted_password_for_https() {
    assert_eq!(
        remote_git_source("https://user:pass@example.com/repo.git"),
        "https://user:redacted@example.com/repo.git"
    );
}

/// NOT INTENDED (known bug), see [`sources_record_a_redacted_password_for_https`]
#[test]
fn sources_record_a_redacted_password_for_ssh() {
    assert_eq!(
        remote_git_source("ssh://user:pass@example.com/repo.git"),
        "ssh://user:redacted@example.com/repo.git"
    );
}

/// NOT INTENDED (known bug), see [`sources_record_a_redacted_password_for_https`]
#[test]
fn sources_record_a_redacted_password_for_git() {
    assert_eq!(
        remote_git_source("git://user:pass@example.com/repo.git"),
        "git://user:redacted@example.com/repo.git"
    );
}

/// Intended: `gix::Url` does not redact `file://` URLs, so the source is
/// the URL as written (password included; the URL came from a
/// non-secret project or config file).
#[test]
fn sources_keep_userinfo_verbatim_for_file() {
    assert_eq!(
        remote_git_source("file://user:pass@example.com/repo"),
        "file://user:pass@example.com/repo"
    );
}
