#![allow(unused_imports)]
use std::{io::Read, process::Command};

use assert_cmd::prelude::*;
use tempfile::tempdir;

use crate::project::{ProjectRead, gix_git_download::GixDownloadedProject};
//use predicates::prelude::*;

/// Initializes a git repository at `path` with a pre-configured test user.
#[cfg(feature = "alltests")]
fn git_init(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    Command::new("git")
        .arg("init")
        .current_dir(path)
        .output()?
        .assert()
        .success();
    Command::new("git")
        .args(["config", "user.email", "user@sysand.org"])
        .current_dir(path)
        .output()?
        .assert()
        .success();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()?
        .assert()
        .success();
    Ok(())
}

#[cfg(feature = "alltests")]
#[test]
pub fn basic_gix_access() -> Result<(), Box<dyn std::error::Error>> {
    let repo_dir = tempdir()?;
    git_init(repo_dir.path())?;

    // TODO: Replace by commands::*::do_* when sufficiently complete, also use gix to create repo?
    std::fs::write(
        repo_dir.path().join(".project.json"),
        r#"{"name":"basic_gix_access","version":"1.2.3","usage":[]}"#,
    )?;
    Command::new("git")
        .arg("add")
        .arg(".project.json")
        .current_dir(repo_dir.path())
        .output()?
        .assert()
        .success();

    std::fs::write(
        repo_dir.path().join(".meta.json"),
        r#"{"index":{},"created":"123"}"#,
    )?;
    Command::new("git")
        .arg("add")
        .arg(".meta.json")
        .current_dir(repo_dir.path())
        .output()?
        .assert()
        .success();

    std::fs::write(repo_dir.path().join("test.sysml"), "package Test;")?;
    Command::new("git")
        .arg("add")
        .arg("test.sysml")
        .current_dir(repo_dir.path())
        .output()?
        .assert()
        .success();

    Command::new("git")
        .args(["commit", "-m", "test_commit"])
        .current_dir(repo_dir.path())
        .output()?
        .assert()
        .success();

    Command::new("git")
        .arg("update-server-info")
        .current_dir(repo_dir.path())
        .output()?
        .assert()
        .success();

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

    let canonical = repo_dir.path().canonicalize()?;
    // On Windows, canonicalize() returns extended-length paths with a `\\?\`
    // prefix that gix cannot parse as a valid file URL. Strip it.
    let path = canonical.to_str().unwrap();
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
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
