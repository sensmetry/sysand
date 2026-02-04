// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use predicates::prelude::*;
use sysand_core::{
    model::{InterchangeProjectChecksumRaw, KerMlChecksumAlg},
    project::{ProjectRead, local_kpar::LocalKParProject},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn test_project_build() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        run_sysand(["init", "--version", "1.2.3", "--name", "test_build"], None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["info", "--path", cwd.join("test_build.kpar").as_str()],
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
        &InterchangeProjectChecksumRaw {
            value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8".to_string(),
            algorithm: KerMlChecksumAlg::Sha256.into()
        }
    );

    assert_eq!(meta.index.len(), 1);
    assert_eq!(meta.index.get("P").unwrap(), "test.sysml");

    Ok(())
}

#[test]
fn test_workspace_build() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project_group_cwd = cwd.join("subgroup");
    std::fs::create_dir(&project_group_cwd)?;
    let project1_cwd = project_group_cwd.join("project1");
    let project2_cwd = project_group_cwd.join("project2");
    let project3_cwd = cwd.join("project3");

    // Create .workspace.json file
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [
            {"path": "subgroup/project1", "iris": ["urn:kpar:project1"]},
            {"path": "subgroup/project2", "iris": ["urn:kpar:project2"]},
            {"path": "project3", "iris": ["urn:kpar:project3"]}
            ]}"#,
    )?;

    for project_cwd in [&project1_cwd, &project2_cwd, &project3_cwd] {
        std::fs::create_dir(project_cwd)?;
        let project_name = project_cwd.file_name().unwrap();
        let out = run_sysand_in(
            project_cwd,
            ["init", "--version", "1.2.3", "--name", project_name],
            None,
        )?;
        out.assert().success();

        std::fs::write(project_cwd.join("test.sysml"), b"package P;\n")?;
        let out = run_sysand_in(
            project_cwd,
            ["include", "--no-index-symbols", "test.sysml"],
            None,
        )?;
        out.assert().success();
    }

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    for project_name in ["project1", "project2", "project3"] {
        println!("W9: {}", project_name);
        let kpar_path = cwd
            .join("output")
            .join(format!("{}-1.2.3.kpar", project_name));
        assert!(
            kpar_path.is_file(),
            "kpar file does not exist: {}",
            kpar_path
        );

        let out = run_sysand_in(&cwd, ["info", "--path", kpar_path.as_str()], None)?;

        out.assert()
            .success()
            .stdout(predicate::str::contains(format!("Name: {}", project_name)))
            .stdout(predicate::str::contains("Version: 1.2.3"));

        let kpar_project = LocalKParProject::new_guess_root(kpar_path)?;

        let (Some(_), Some(meta)) = kpar_project.get_project()? else {
            panic!("failed to get built project info/meta");
        };

        // Ensure things get canonicalised during build

        assert_eq!(meta.checksum.as_ref().unwrap().len(), 1);
        assert_eq!(
            meta.checksum.as_ref().unwrap().get("test.sysml").unwrap(),
            &InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        );

        assert_eq!(meta.index.len(), 1);
        assert_eq!(meta.index.get("P").unwrap(), "test.sysml");
    }

    Ok(())
}
