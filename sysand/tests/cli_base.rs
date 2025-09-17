// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn basic_execution() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(&vec!["--version"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("sysand"));

    Ok(())
}
