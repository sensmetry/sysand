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

#[test]
fn include_expands_glob_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "windows_glob", "1.0.0")?;

    out.assert().success();

    fs::write(cwd.join("p1.sysml"), b"package P1;\n")?;
    fs::write(cwd.join("p2.sysml"), b"package P2;\n")?;
    fs::write(cwd.join("p3.sysml"), b"package P3;\n")?;
    // Should not be picked up by the `*.sysml` glob below.
    fs::write(cwd.join("readme.txt"), b"not a model file\n")?;

    // Passed as a single, unexpanded argument, exactly as `cmd.exe` and
    // PowerShell would hand it to us.
    let out = run_sysand_in(&cwd, ["include", "*.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        IndexMap::from([
            ("P1".to_owned(), "p1.sysml".to_owned()),
            ("P2".to_owned(), "p2.sysml".to_owned()),
            ("P3".to_owned(), "p3.sysml".to_owned()),
        ])
    );

    Ok(())
}
