// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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

/// Workspace `meta.metamodel` is NOT auto-injected — projects must explicitly
/// inherit via `{ "workspace": true }` in `.meta.json`.
#[test]
fn workspace_build_metamodel_not_auto_injected() -> Result<(), Box<dyn std::error::Error>> {
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
    assert!(
        meta.metamodel.is_none(),
        "metamodel should be None when project does not explicitly inherit it"
    );

    Ok(())
}

/// Project inherits metamodel from workspace root via `{ "workspace": true }` in `.meta.json`.
#[test]
fn workspace_inherit_metamodel_from_root_in_meta_json() -> Result<(), Box<dyn std::error::Error>> {
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

    // Explicitly inherit metamodel from workspace root
    let meta_path = project1_cwd.join(".meta.json");
    let meta_content = std::fs::read_to_string(&meta_path)?;
    let mut meta_json: serde_json::Value = serde_json::from_str(&meta_content)?;
    meta_json["metamodel"] = serde_json::json!({"preset": "default"});
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_json)?)?;

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

#[test]
fn compression_methods() -> Result<(), Box<dyn std::error::Error>> {
    let compressions = KparCompressionMethodCli::value_variants();
    compression_method(None)?;
    for compression in compressions {
        compression_method(Some(compression.to_possible_value().unwrap().get_name()))?;
    }
    Ok(())
}

/// Build a project with a README.md at the project root
#[test]
fn project_build_with_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_readme"],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::write(cwd.join("README.md"), b"# My Project\nHello world\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including readme from"));

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "README.md",
        "# My Project\nHello world\n",
    );

    Ok(())
}

/// Build a project without any README file — should succeed
#[test]
fn project_build_without_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_no_readme"],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_missing(&cwd.join("test_build.kpar"), "README.md");

    Ok(())
}

/// Build workspace with per-project READMEs
#[test]
fn workspace_build_with_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");
    let project2_cwd = cwd.join("project2");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [
            {"path": "project1", "iris": ["urn:kpar:project1"]},
            {"path": "project2", "iris": ["urn:kpar:project2"]}
            ]}"#,
    )?;

    for (project_cwd, readme_content) in [
        (&project1_cwd, "# Project 1\n"),
        (&project2_cwd, "# Project 2\n"),
    ] {
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

        std::fs::write(project_cwd.join("README.md"), readme_content.as_bytes())?;
    }

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    for (project_name, expected_readme) in
        [("project1", "# Project 1\n"), ("project2", "# Project 2\n")]
    {
        let kpar_path = cwd
            .join("output")
            .join(format!("{}-1.2.3.kpar", project_name));
        assert!(
            kpar_path.is_file(),
            "kpar file does not exist: {}",
            kpar_path
        );

        assert_kpar_file(&kpar_path, "README.md", expected_readme);
    }

    Ok(())
}

/// Build a project with a CHANGELOG.md at the project root
#[test]
fn project_build_with_changelog() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_changelog"],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::write(
        cwd.join("CHANGELOG.md"),
        b"# Changelog\n\n## 1.2.3\n- Initial release\n",
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including changelog from"));

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "CHANGELOG.md",
        "# Changelog\n\n## 1.2.3\n- Initial release\n",
    );

    Ok(())
}

/// Build a project without any CHANGELOG file — should succeed
#[test]
fn project_build_without_changelog() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_no_changelog"],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_missing(&cwd.join("test_build.kpar"), "CHANGELOG.md");

    Ok(())
}

/// Build workspace with per-project CHANGELOGs
#[test]
fn workspace_build_with_changelog() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");
    let project2_cwd = cwd.join("project2");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [
            {"path": "project1", "iris": ["urn:kpar:project1"]},
            {"path": "project2", "iris": ["urn:kpar:project2"]}
            ]}"#,
    )?;

    for (project_cwd, changelog_content) in [
        (&project1_cwd, "# Project 1 Changelog\n"),
        (&project2_cwd, "# Project 2 Changelog\n"),
    ] {
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

        std::fs::write(
            project_cwd.join("CHANGELOG.md"),
            changelog_content.as_bytes(),
        )?;
    }

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    for (project_name, expected_changelog) in [
        ("project1", "# Project 1 Changelog\n"),
        ("project2", "# Project 2 Changelog\n"),
    ] {
        let kpar_path = cwd
            .join("output")
            .join(format!("{}-1.2.3.kpar", project_name));
        assert!(
            kpar_path.is_file(),
            "kpar file does not exist: {}",
            kpar_path
        );

        assert_kpar_file(&kpar_path, "CHANGELOG.md", expected_changelog);
    }

    Ok(())
}

/// Build a project with a single SPDX license — the matching
/// `LICENSES/<id>.txt` is included in the KPAR.
#[test]
fn project_build_with_single_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "test_license",
            "--license",
            "MIT",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("MIT.txt"), b"MIT license text\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including license from"));

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/MIT.txt",
        "MIT license text\n",
    );

    Ok(())
}

/// Build a project with a compound SPDX expression — both license files
/// are included.
#[test]
fn project_build_with_compound_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "test_compound_license",
            "--license",
            "MIT OR Apache-2.0",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("MIT.txt"), b"MIT body\n")?;
    std::fs::write(
        cwd.join("LICENSES").join("Apache-2.0.txt"),
        b"Apache body\n",
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/MIT.txt",
        "MIT body\n",
    );
    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/Apache-2.0.txt",
        "Apache body\n",
    );

    Ok(())
}

/// Build a project with a `WITH` exception — both the license and the
/// exception file are included.
#[test]
fn project_build_with_license_exception() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "test_license_with",
            "--license",
            "GPL-2.0-only WITH Classpath-exception-2.0",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("GPL-2.0-only.txt"), b"GPL body\n")?;
    std::fs::write(
        cwd.join("LICENSES").join("Classpath-exception-2.0.txt"),
        b"Classpath exception body\n",
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/GPL-2.0-only.txt",
        "GPL body\n",
    );
    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/Classpath-exception-2.0.txt",
        "Classpath exception body\n",
    );

    Ok(())
}

/// Build a project with a custom `LicenseRef-` identifier — the matching
/// file is bundled verbatim.
#[test]
fn project_build_with_license_ref() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "test_license_ref",
            "--license",
            "LicenseRef-MyCustom",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(
        cwd.join("LICENSES").join("LicenseRef-MyCustom.txt"),
        b"Custom license body\n",
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test_build.kpar"),
        "LICENSES/LicenseRef-MyCustom.txt",
        "Custom license body\n",
    );

    Ok(())
}

/// Build a project that declares a license but ships no matching file —
/// the build succeeds and warns about the missing license file.
#[test]
fn project_build_with_license_file_missing() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "test_missing_license",
            "--license",
            "MIT",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("LICENSES/MIT.txt"));

    assert_kpar_missing(&cwd.join("test_build.kpar"), "LICENSES/MIT.txt");

    Ok(())
}

/// Build a project without any license — no LICENSES/* entries are added.
#[test]
fn project_build_without_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--version", "1.2.3", "--name", "test_no_license"],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test_build.kpar"], None)?;
    out.assert().success();

    assert_kpar_no_licenses_dir(&cwd.join("test_build.kpar"));

    Ok(())
}

fn assert_kpar_file(kpar_path: &camino::Utf8Path, archive_path: &str, expected: &str) {
    let file = std::fs::File::open(kpar_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut entry = archive
        .by_name(archive_path)
        .unwrap_or_else(|_| panic!("expected {archive_path} in {kpar_path}"));
    let mut content = String::new();
    entry.read_to_string(&mut content).unwrap();
    assert_eq!(content, expected, "{archive_path} mismatch in {kpar_path}");
}

fn assert_kpar_missing(kpar_path: &camino::Utf8Path, archive_path: &str) {
    let file = std::fs::File::open(kpar_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(
        archive.by_name(archive_path).is_err(),
        "KPAR should not contain {archive_path}: {kpar_path}"
    );
}

fn assert_kpar_no_licenses_dir(kpar_path: &camino::Utf8Path) {
    let file = std::fs::File::open(kpar_path).unwrap();
    let archive = zip::ZipArchive::new(file).unwrap();
    for name in archive.file_names() {
        assert!(
            !name.starts_with("LICENSES/"),
            "KPAR should not contain any LICENSES/ entries but found {name}: {kpar_path}"
        );
    }
}

fn compression_method(compression: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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

/// Helper: create a minimal workspace project with a .sysml source file.
///
/// Initialises the project with a placeholder version, runs `include` to
/// create `.meta.json`, then replaces `.project.json` with the final content
/// (which may contain workspace inheritance references that commands other
/// than `build` cannot resolve).
fn setup_workspace_project(
    project_cwd: &camino::Utf8Path,
    project_name: &str,
    final_project_json: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir(project_cwd)?;
    std::fs::write(project_cwd.join("test.sysml"), b"package P;\n")?;

    // Init with a placeholder version so `include` can read a valid .project.json.
    run_sysand_in(
        project_cwd,
        ["init", "--version", "0.0.0", "--name", project_name],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        project_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?
    .assert()
    .success();

    // Now overwrite with the final .project.json (may contain workspace refs).
    std::fs::write(project_cwd.join(".project.json"), final_project_json)?;
    Ok(())
}

/// Workspace project inherits version from workspace root `project` defaults
/// using `"version": { "preset": "default" }`.
#[test]
fn workspace_inherit_version_from_root() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "project": {
                "version": "3.0.0"
            }
        }"#,
    )?;

    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": {"preset": "default"}, "usage": []}"#,
    )?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-3.0.0.kpar");
    assert!(kpar_path.is_file(), "expected output/project1-3.0.0.kpar");

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (Some(info), _) = kpar_project.get_project()? else {
        panic!("failed to get project info");
    };
    assert_eq!(info.version, "3.0.0");

    Ok(())
}

/// Workspace project inherits version from a named preset. Metamodel is NOT
/// implicitly inherited — it must be explicitly referenced in `.meta.json`.
#[test]
fn workspace_inherit_version_from_group() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "presets": {
                "kerml": {
                    "project": { "version": "1.0.0" },
                    "meta": { "metamodel": "https://www.omg.org/spec/KerML/20250201" }
                }
            }
        }"#,
    )?;

    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": {"preset": "kerml"}, "usage": []}"#,
    )?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(kpar_path.is_file(), "expected output/project1-1.0.0.kpar");

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (Some(info), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get project info/meta");
    };
    assert_eq!(info.version, "1.0.0");
    assert!(
        meta.metamodel.is_none(),
        "metamodel should be None when not explicitly inherited"
    );

    Ok(())
}

/// `.meta.json` with `"metamodel": { "preset": "kerml" }` resolves the
/// metamodel from the named preset.
#[test]
fn workspace_inherit_metamodel_from_group_in_meta_json() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "presets": {
                "kerml": {
                    "project": { "version": "1.0.0" },
                    "meta": { "metamodel": "https://www.omg.org/spec/KerML/20250201" }
                }
            }
        }"#,
    )?;

    // Version is literal; only metamodel inherits from the preset via .meta.json
    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": "2.0.0", "usage": []}"#,
    )?;
    // Overwrite the generated .meta.json with a preset metamodel reference
    let meta_path = project1_cwd.join(".meta.json");
    let meta_content = std::fs::read_to_string(&meta_path)?;
    let mut meta_json: serde_json::Value = serde_json::from_str(&meta_content)?;
    meta_json["metamodel"] = serde_json::json!({"preset": "kerml"});
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta_json)?)?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-2.0.0.kpar");
    assert!(kpar_path.is_file(), "expected output/project1-2.0.0.kpar");

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (_, Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get project meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/KerML/20250201")
    );

    Ok(())
}

/// Referencing an unknown workspace preset reports a clear error.
#[test]
fn workspace_inherit_unknown_group_error() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ]
        }"#,
    )?;

    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": {"preset": "nonexistent"}, "usage": []}"#,
    )?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("nonexistent"));

    Ok(())
}

/// Workspace project with inherited publisher and license from root defaults.
#[test]
fn workspace_inherit_publisher_and_license_from_root() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "project": {
                "version": "1.0.0",
                "publisher": "Acme Corp",
                "license": "Apache-2.0"
            }
        }"#,
    )?;

    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": {"preset": "default"}, "publisher": {"preset": "default"}, "license": {"preset": "default"}, "usage": []}"#,
    )?;

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(kpar_path.is_file(), "expected output/project1-1.0.0.kpar");

    let kpar_project = LocalKParProject::new_guess_root(&kpar_path)?;
    let (Some(info), _) = kpar_project.get_project()? else {
        panic!("failed to get project info");
    };
    assert_eq!(info.version, "1.0.0");
    assert_eq!(info.publisher.as_deref(), Some("Acme Corp"));
    assert_eq!(info.license.as_deref(), Some("Apache-2.0"));

    Ok(())
}

/// Workspace inheritance is idempotent — building twice succeeds.
#[test]
fn workspace_inherit_version_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let project1_cwd = cwd.join("project1");

    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{
            "projects": [
                {"path": "project1", "iris": ["urn:kpar:project1"]}
            ],
            "project": { "version": "5.0.0" }
        }"#,
    )?;

    setup_workspace_project(
        &project1_cwd,
        "project1",
        br#"{"name": "project1", "version": {"preset": "default"}, "usage": []}"#,
    )?;

    // First build
    run_sysand_in(&cwd, ["build"], None)?.assert().success();
    // Second build — must also succeed
    run_sysand_in(&cwd, ["build"], None)?.assert().success();

    // Verify original .project.json was NOT modified (still has preset ref)
    let original_content = std::fs::read_to_string(project1_cwd.join(".project.json"))?;
    let original: serde_json::Value = serde_json::from_str(&original_content)?;
    assert_eq!(
        original["version"],
        serde_json::json!({"preset": "default"}),
        "original .project.json should still contain the preset reference"
    );

    Ok(())
}
