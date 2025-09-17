// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Write;

use assert_cmd::prelude::*;
use predicates::prelude::*;
use sysand_core::{
    model::InterchangeProjectChecksum,
    project::{ProjectRead, local_kpar::LocalKParProject},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn test_build() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        &vec!["init", "--version", "1.2.3", "--name", "test_build"],
        None,
    )?;

    {
        let mut sysml_file = std::fs::File::create(cwd.join("test.sysml"))?;
        sysml_file.write_all(b"package P;\n")?;
    }

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        &vec!["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, &vec!["build", "./test_build.kpar"], None)?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        &vec![
            "info",
            "--path",
            &cwd.join("test_build.kpar").to_string_lossy(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: test_build"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let kpar_project = LocalKParProject::new_guess_root(cwd.join("test_build.kpar"))?;

    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    // Ensure things get canonicalised during build

    assert_eq!(meta.checksum.as_ref().unwrap().len(), 1);
    assert_eq!(
        meta.checksum.as_ref().unwrap().get("test.sysml").unwrap(),
        &InterchangeProjectChecksum {
            value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8".to_string(),
            algorithm: "SHA256".to_string(),
        }
    );

    assert_eq!(meta.index.len(), 1);
    assert_eq!(meta.index.get("P").unwrap(), "test.sysml");

    Ok(())
}
