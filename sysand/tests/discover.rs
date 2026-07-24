// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand new` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, falling back
/// on directory name as name.
#[test]
fn discover_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) =
        cli_init_project(Some("discover_basic"), "a", None, Some("1.2.3"), None)?;

    let project_path = cwd.join("discover_basic").join("path");

    std::fs::create_dir(&project_path)?;

    let out_1 = run_sysand_in(&project_path, ["print-root"], None)?;

    out_1
        .assert()
        .success()
        .stdout(predicate::str::contains(cwd));

    Ok(())
}
