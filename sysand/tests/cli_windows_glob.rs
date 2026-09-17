// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Unlike POSIX shells, `cmd.exe` and PowerShell do not expand glob patterns
//! (`*`, `?`, ...) in arguments before invoking a program; expansion is left
//! up to the program itself.

#![cfg(target_os = "windows")]

use std::fs;

use assert_cmd::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn include_expands_glob_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("acme", "windows-glob", "1.0.0")?;

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

    // `init` writes a `model.toml`, so the glob's result is in
    // `sources` and there is no `.meta.json` symbol index. This file is
    // `cfg`'d to Windows, so only a Windows CI run compiles it.
    let manifest = fs::read_to_string(cwd.join("model.toml"))?;
    assert!(!cwd.join(".meta.json").exists(), "`init` writes no json");

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
