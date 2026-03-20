// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use assert_cmd::prelude::*;
use clap::ValueEnum;
use predicates::prelude::*;
use std::io::{Read, Write};
use sysand::cli::KparCompressionMethodCli;
use sysand_core::{
    model::{InterchangeProjectChecksumRaw, KerMlChecksumAlg},
    project::{ProjectRead, local_kpar::LocalKParProject},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn project_build() -> Result<(), Box<dyn std::error::Error>> {
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

/// Build a project that has a path (`file:`) usage
#[test]
fn project_build_path_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1, out1) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_build1"],
        None,
    )?;
    let (_temp_dir2, cwd2, out2) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_build2"],
        None,
    )?;

    out1.assert().success();
    out2.assert().success();

    let out = run_sysand_in(&cwd1, ["add", "--path", cwd2.as_str()], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd1, ["build", "./test_build.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("project includes a path usage"));
    assert!(!cwd1.join("test_build.kpar").exists());

    let out = run_sysand_in(
        &cwd1,
        ["build", "./test_build.kpar", "--allow-path-usage"],
        None,
    )?;

    // Warning must still be produced
    out.assert()
        .success()
        .stderr(predicate::str::contains("project includes a path usage"));

    let out = run_sysand_in(
        &cwd1,
        ["info", "--path", cwd1.join("test_build.kpar").as_str()],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: test_build"))
        .stdout(predicate::str::contains("Version: 1.2.3"))
        .stdout(predicate::str::contains(file_url_from_path(&cwd2)));

    let kpar_project = LocalKParProject::new_guess_root(cwd1.join("test_build.kpar"))?;

    let (Some(_), Some(_)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    Ok(())
}

#[test]
fn workspace_build() -> Result<(), Box<dyn std::error::Error>> {
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

/// Workspace with `meta.metamodel` set — projects without metamodel get it injected
#[test]
fn workspace_build_with_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20250201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    let out = run_sysand_in(
        &project1_cwd,
        ["init", "--version", "1.0.0", "--name", "project1"],
        None,
    )?;
    out.assert().success();

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(
        kpar_path.is_file(),
        "kpar file does not exist: {}",
        kpar_path
    );

    let kpar_project = LocalKParProject::new_guess_root(kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );

    Ok(())
}

/// Workspace with unknown `meta.metamodel` — build succeeds with a warning
#[test]
fn workspace_build_with_unknown_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20251201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    let out = run_sysand_in(
        &project1_cwd,
        ["init", "--version", "1.0.0", "--name", "project1"],
        None,
    )?;
    out.assert().success();

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("unknown metamodel"));

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(
        kpar_path.is_file(),
        "kpar file does not exist: {}",
        kpar_path
    );

    let kpar_project = LocalKParProject::new_guess_root(kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20251201")
    );

    Ok(())
}

/// Workspace with `meta.metamodel` + project that also has metamodel — build fails
#[test]
fn workspace_build_metamodel_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20250201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    let out = run_sysand_in(
        &project1_cwd,
        ["init", "--version", "1.0.0", "--name", "project1"],
        None,
    )?;
    out.assert().success();

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    // Set metamodel in the project's .meta.json to create a conflict
    let meta_path = project1_cwd.join(".meta.json");
    let meta_content = std::fs::read_to_string(&meta_path)?;
    let mut meta: serde_json::Value = serde_json::from_str(&meta_content)?;
    meta["metamodel"] =
        serde_json::Value::String("https://www.omg.org/spec/KerML/20250201".to_string());
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("sets a different metamodel"));

    Ok(())
}

/// Workspace and project set the **same** metamodel — no conflict, build succeeds
#[test]
fn workspace_build_metamodel_same_no_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20250201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    let out = run_sysand_in(
        &project1_cwd,
        ["init", "--version", "1.0.0", "--name", "project1"],
        None,
    )?;
    out.assert().success();

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    // Set the same metamodel in the project's .meta.json — should NOT conflict
    let meta_path = project1_cwd.join(".meta.json");
    let meta_content = std::fs::read_to_string(&meta_path)?;
    let mut meta: serde_json::Value = serde_json::from_str(&meta_content)?;
    meta["metamodel"] =
        serde_json::Value::String("https://www.omg.org/spec/SysML/20250201".to_string());
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    Ok(())
}

/// Building a workspace with `meta.metamodel` twice must succeed both times.
/// This verifies that `put_meta` only writes to the temp directory, not the
/// original project directory, so the conflict check sees the same unmodified
/// `.meta.json` on both runs.
#[test]
fn workspace_build_metamodel_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20250201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    let out = run_sysand_in(
        &project1_cwd,
        ["init", "--version", "1.0.0", "--name", "project1"],
        None,
    )?;
    out.assert().success();

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    // First build
    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(kpar_path.is_file());

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );

    // Second build — must also succeed (no conflict from first build's injection)
    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta on second build");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );

    // Verify original project .meta.json was NOT modified
    let original_meta_content = std::fs::read_to_string(project1_cwd.join(".meta.json"))?;
    let original_meta: serde_json::Value = serde_json::from_str(&original_meta_content)?;
    assert!(
        original_meta.get("metamodel").is_none(),
        "original project .meta.json should not have metamodel set"
    );

    Ok(())
}

#[test]
fn test_compression_methods() -> Result<(), Box<dyn std::error::Error>> {
    let compressions = KparCompressionMethodCli::value_variants();
    test_compression_method(None)?;
    for compression in compressions {
        test_compression_method(Some(compression.to_possible_value().unwrap().get_name()))?;
    }
    Ok(())
}

fn test_compression_method(compression: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        run_sysand(["init", "--version", "1.2.3", "--name", "test_build"], None)?;

    {
        let mut sysml_file = std::fs::File::create(cwd.join("test.sysml"))?;
        sysml_file.write_all(b"package P;\n")?;
    }

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;

    out.assert().success();

    let out = match compression {
        Some(compression) => run_sysand_in(
            &cwd,
            ["build", "--compression", compression, "./test_build.kpar"],
            None,
        )?,
        None => run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?,
    };

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

    let (Some(info), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    assert_eq!(info.name, "test_build");
    assert_eq!(info.version, "1.2.3");

    assert_eq!(meta.checksum.as_ref().unwrap().len(), 1);
    assert_eq!(meta.index.len(), 1);
    assert_eq!(meta.index.get("P").unwrap(), "test.sysml");
    let mut src = String::new();
    kpar_project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package P;\n");
    Ok(())
}
