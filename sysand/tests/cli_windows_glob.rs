// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Unlike POSIX shells, `cmd.exe` and PowerShell do not expand glob patterns
//! (`*`, `?`, ...) in arguments before invoking a program; expansion is left
//! up to the program itself.

#![cfg(target_os = "windows")]

use std::fs;

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use sysand_core::model::InterchangeProjectMetadataRaw;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// Runs `include '*.sysml'` in `cwd` as one unexpanded argument, exactly as
/// `cmd.exe` and PowerShell would hand it to us.
fn include_glob(cwd: &camino::Utf8Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(cwd.join("p1.sysml"), b"package P1;\n")?;
    fs::write(cwd.join("p2.sysml"), b"package P2;\n")?;
    fs::write(cwd.join("p3.sysml"), b"package P3;\n")?;
    // Should not be picked up by the glob.
    fs::write(cwd.join("readme.txt"), b"not a model file\n")?;

    run_sysand_in(cwd, ["include", "*.sysml"], None)?
        .assert()
        .success();
    Ok(())
}

#[test]
fn include_expands_glob_pattern_toml() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--kind",
            "model-toml",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "windows-glob",
            "--version",
            "1.0.0",
        ],
        None,
    )?
    .assert()
    .success();

    include_glob(&cwd)?;

    let manifest = fs::read_to_string(cwd.join("model.toml"))?;
    for expected in ["\"p1.sysml\"", "\"p2.sysml\"", "\"p3.sysml\""] {
        assert!(
            manifest.contains(expected),
            "the glob expanded to {expected}:\n{manifest}"
        );
    }
    assert!(
        !manifest.contains("readme.txt"),
        "`*.sysml` does not match a `.txt`:\n{manifest}"
    );
    Ok(())
}

#[test]
fn include_expands_glob_pattern_json() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "windows-glob", "1.0.0", None)?;

    include_glob(&cwd)?;

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;
    assert_eq!(
        meta.index,
        Some(IndexMap::from([
            ("P1".to_owned(), "p1.sysml".to_owned()),
            ("P2".to_owned(), "p2.sysml".to_owned()),
            ("P3".to_owned(), "p3.sysml".to_owned()),
        ]))
    );
    Ok(())
}
