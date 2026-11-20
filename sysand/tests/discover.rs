// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand init` creates a project in the specified directory, falling back
/// on the directory name as name, and `print-root` finds it from a
/// subdirectory.
#[test]
fn discover_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) =
        cli_init_project(Some("discover-basic"), "acme", None, Some("1.2.3"), None)?;

    let project_path = cwd.join("discover-basic").join("path");

    std::fs::create_dir(&project_path)?;

    let out_1 = run_sysand_in(&project_path, ["print-root"], None)?;

    out_1
        .assert()
        .success()
        .stdout(predicate::str::contains(cwd));

    Ok(())
}
