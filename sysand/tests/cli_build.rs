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

/// Create a minimal buildable project (init + write sysml + include).
fn init_buildable_project(
    name: &str,
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(["init", "--version", "1.2.3", "--name", name], None)?;
    out.assert().success();
    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();
    Ok((_temp_dir, cwd))
}

/// Assert that a kpar archive contains (or does not contain) a README.md with the expected content.
fn assert_kpar_readme(
    kpar_path: &camino::Utf8Path,
    expected: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(kpar_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    match expected {
        Some(expected_content) => {
            let mut readme = archive.by_name("README.md")?;
            let mut content = String::new();
            readme.read_to_string(&mut content)?;
            assert_eq!(content, expected_content);
        }
        None => {
            assert!(
                archive.by_name("README.md").is_err(),
                "KPAR should not contain README.md"
            );
        }
    }
    Ok(())
}

/// Set up a two-project workspace, calling `per_project` for each project directory.
fn init_workspace(
    per_project: impl Fn(&camino::Utf8Path, &str) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [
            {"path": "project1", "iris": ["urn:kpar:project1"]},
            {"path": "project2", "iris": ["urn:kpar:project2"]}
            ]}"#,
    )?;

    for project_name in ["project1", "project2"] {
        let project_cwd = cwd.join(project_name);
        std::fs::create_dir(&project_cwd)?;
        let out = run_sysand_in(
            &project_cwd,
            ["init", "--version", "1.2.3", "--name", project_name],
            None,
        )?;
        out.assert().success();
        std::fs::write(project_cwd.join("test.sysml"), b"package P;\n")?;
        let out = run_sysand_in(
            &project_cwd,
            ["include", "--no-index-symbols", "test.sysml"],
            None,
        )?;
        out.assert().success();
        per_project(&project_cwd, project_name)?;
    }

    Ok((_temp_dir, cwd))
}

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

#[test]
fn test_compression_methods() -> Result<(), Box<dyn std::error::Error>> {
    let compressions = KparCompressionMethodCli::value_variants();
    test_compression_method(None)?;
    for compression in compressions {
        test_compression_method(Some(compression.to_possible_value().unwrap().get_name()))?;
    }
    Ok(())
}

/// Build a project with a README.md at the project root
#[test]
fn project_build_with_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_buildable_project("test_readme")?;
    std::fs::write(cwd.join("README.md"), b"# My Project\nHello world\n")?;

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including readme"));

    assert_kpar_readme(
        &cwd.join("test_build.kpar"),
        Some("# My Project\nHello world\n"),
    )?;
    Ok(())
}

/// Build a project with a custom README filename via config
#[test]
fn project_build_with_custom_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_buildable_project("test_custom_readme")?;
    std::fs::write(
        cwd.join("PUBLIC_README.md"),
        b"# Public Readme\nCustom content\n",
    )?;

    let cfg_path = cwd.join("sysand.toml");
    std::fs::write(&cfg_path, b"[build]\nreadme = \"PUBLIC_README.md\"\n")?;

    let out = run_sysand_in(
        &cwd,
        ["build", "./test_build.kpar"],
        Some(cfg_path.as_str()),
    )?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including readme"));

    assert_kpar_readme(
        &cwd.join("test_build.kpar"),
        Some("# Public Readme\nCustom content\n"),
    )?;
    Ok(())
}

/// Build a project without any README file — should succeed
#[test]
fn project_build_without_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_buildable_project("test_no_readme")?;

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_readme(&cwd.join("test_build.kpar"), None)?;
    Ok(())
}

/// Build workspace with per-project READMEs
#[test]
fn workspace_build_with_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_workspace(|project_cwd, name| {
        std::fs::write(
            project_cwd.join("README.md"),
            format!("# {name}\n").as_bytes(),
        )?;
        Ok(())
    })?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    for name in ["project1", "project2"] {
        let kpar_path = cwd.join("output").join(format!("{name}-1.2.3.kpar"));
        assert_kpar_readme(&kpar_path, Some(&format!("# {name}\n")))?;
    }
    Ok(())
}

/// Build workspace with a custom README path configured via sysand.toml
#[test]
fn workspace_build_with_custom_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_workspace(|project_cwd, name| {
        std::fs::write(
            project_cwd.join("CUSTOM_README.md"),
            format!("# {name} Custom\n").as_bytes(),
        )?;
        std::fs::write(
            project_cwd.join("sysand.toml"),
            b"[build]\nreadme = \"CUSTOM_README.md\"\n",
        )?;
        Ok(())
    })?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    for name in ["project1", "project2"] {
        let kpar_path = cwd.join("output").join(format!("{name}-1.2.3.kpar"));
        assert_kpar_readme(&kpar_path, Some(&format!("# {name} Custom\n")))?;
    }
    Ok(())
}

/// Build a project with `readme = false` — should NOT bundle README even if file exists
#[test]
fn project_build_readme_disabled() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_buildable_project("test_readme_disabled")?;
    std::fs::write(cwd.join("README.md"), b"# Should not be bundled\n")?;

    let cfg_path = cwd.join("sysand.toml");
    std::fs::write(&cfg_path, b"[build]\nreadme = false\n")?;

    let out = run_sysand_in(
        &cwd,
        ["build", "./test_build.kpar"],
        Some(cfg_path.as_str()),
    )?;
    out.assert().success();

    assert_kpar_readme(&cwd.join("test_build.kpar"), None)?;
    Ok(())
}

/// Build with a required readme (explicit path) that doesn't exist — should fail
#[test]
fn project_build_required_readme_missing_file() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_buildable_project("test_readme_missing")?;
    // Deliberately NOT creating the readme file

    let cfg_path = cwd.join("sysand.toml");
    std::fs::write(&cfg_path, b"[build]\nreadme = \"NONEXISTENT.md\"\n")?;

    let out = run_sysand_in(
        &cwd,
        ["build", "./test_build.kpar"],
        Some(cfg_path.as_str()),
    )?;
    out.assert().failure();

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
