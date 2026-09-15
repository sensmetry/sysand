// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! Every project in this file is written as a legacy `.project.json`
//! pair by hand rather than created through `sysand init`, which
//! writes a `model.toml`: what these fixtures pin is the json path --
//! which "TOML to KPAR" leaves unchanged.
//!
//! The toml path has its own tests at the end of this file, and its
//! usage rules are pinned in `core/src/commands/build_tests.rs`.

use assert_cmd::prelude::*;
use camino::Utf8PathBuf;
use clap::ValueEnum as _;
use indexmap::IndexMap;
use predicates::prelude::*;
use serde_json::json;
use std::{
    fs,
    io::{Read as _, Write as _},
};
use sysand::cli::KparCompressionMethodCli;
use sysand_core::{
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        KerMlChecksumAlg,
    },
    project::{ProjectRead as _, local_kpar::LocalKParProjectRaw},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

// Checksum for file `test.sysml` containing only `package P;`
#[expect(clippy::unnecessary_wraps)]
fn expected_checksum() -> Option<IndexMap<String, InterchangeProjectChecksumRaw>> {
    Some(IndexMap::from([(
        "test.sysml".to_owned(),
        InterchangeProjectChecksumRaw {
            value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8".to_owned(),
            algorithm: KerMlChecksumAlg::Sha256.into(),
        },
    )]))
}

// Index §for file `test.sysml` containing only `package P;`
fn expected_index() -> IndexMap<String, String> {
    IndexMap::from([("P".to_owned(), "test.sysml".to_owned())])
}

/// A key Sysand does not know about is dropped when a KPAR is built, with
/// a warning that says so.
#[test]
fn build_warns_about_an_unknown_key() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-unknown", "1.2.3", None)?;

    let info_path = cwd.join(".project.json");
    let info = fs::read_to_string(&info_path)?;
    fs::write(
        &info_path,
        info.replacen('{', "{\n  \"x-unknown\": \"kept for later\",", 1),
    )?;

    let out = run_sysand_in(&cwd, ["build", "./test-unknown.kpar"], None)?;
    out.assert().success().stderr(predicate::str::contains(
        "unknown key `x-unknown` in `.project.json`",
    ));

    let kpar_project = LocalKParProjectRaw::new_guess_root(cwd.join("test-unknown.kpar"))?;
    let (Some(_), Some(_)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    Ok(())
}

#[test]
fn project_build() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-build", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;

    out.assert().success();
    let orig_info_path = cwd.join(".project.json");
    let orig_meta_path = cwd.join(".meta.json");
    let orig_info_contents = fs::read_to_string(&orig_info_path)?;
    let orig_meta_contents = fs::read_to_string(&orig_meta_path)?;

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar", "--keep-index"], None)?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["info", "--kpar-path", cwd.join("test-build.kpar").as_str()],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: test-build"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let kpar_project = LocalKParProjectRaw::new_guess_root(cwd.join("test-build.kpar"))?;

    let (Some(info), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    // Ensure no changes were made to the original project
    let new_info_contents = fs::read_to_string(&orig_info_path)?;
    let new_meta_contents = fs::read_to_string(&orig_meta_path)?;
    assert_eq!(orig_info_contents, new_info_contents);
    assert_eq!(orig_meta_contents, new_meta_contents);

    // Ensure build artifact info matches original project
    let original_meta: InterchangeProjectMetadataRaw = serde_json::from_str(&orig_meta_contents)?;
    let original_info: InterchangeProjectInfoRaw = serde_json::from_str(&orig_info_contents)?;
    assert_eq!(original_info, info);

    let expected_checksum = expected_checksum();

    // Index should not change
    assert_eq!(meta.index, original_meta.index);
    // File hash should still be updated
    assert_eq!(meta.checksum, expected_checksum);

    // Now canonicalize meta during build
    let out = run_sysand_in(&cwd, ["build", "./test-build2.kpar"], None)?;
    out.assert().success();
    let kpar_project = LocalKParProjectRaw::new_guess_root(cwd.join("test-build2.kpar"))?;

    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    // File hash should be updated
    assert_eq!(meta.checksum, expected_checksum);

    assert_eq!(meta.index, expected_index());

    // Ensure no changes were made to the original project
    let new_info_contents = fs::read_to_string(&orig_info_path)?;
    let new_meta_contents = fs::read_to_string(&orig_meta_path)?;
    assert_eq!(orig_info_contents, new_info_contents);
    assert_eq!(orig_meta_contents, new_meta_contents);

    Ok(())
}

/// `.project.json` and `.meta.json` are pretty-printed (multi-line, indented)
/// inside the built kpar, not minified onto a single line.
#[test]
fn project_build_pretty_prints_project_and_meta_json() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-pretty", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    for archive_path in [".project.json", ".meta.json"] {
        let content = read_kpar_file(&cwd.join("test-build.kpar"), archive_path);

        assert!(
            content.lines().count() > 1,
            "{archive_path} should be pretty-printed across multiple lines, got: {content}"
        );

        let value: serde_json::Value = serde_json::from_str(&content)?;
        let mut expected = serde_json::to_string_pretty(&value)?;
        expected.push('\n');
        assert_eq!(
            content, expected,
            "{archive_path} should use standard pretty-printed formatting"
        );
    }

    Ok(())
}

#[test]
fn build_errors_when_index_symbol_is_missing_from_file() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "test-build-missing-index-symbol",
        "1.2.3",
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package Present;\n")?;
    let out = run_sysand_in(
        &cwd,
        [
            "include",
            "--compute-checksum",
            "--no-index-symbols",
            "test.sysml",
        ],
        None,
    )?;
    out.assert().success();

    let meta_path = cwd.join(".meta.json");
    let mut meta: serde_json::Value = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;
    meta["index"] = json!({
        "Missing": "test.sysml"
    });
    std::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;

    let out = run_sysand_in(&cwd, ["build", "./test_build_good.kpar"], None)?;
    out.assert().success();
    // Build should only fail when symbols are not updated
    let out = run_sysand_in(
        &cwd,
        ["build", "./test_build_bad.kpar", "--keep-index"],
        None,
    )?;
    out.assert().failure().stderr(predicate::str::contains(
        "file `test.sysml` is missing symbol `Missing` found in index",
    ));
    assert!(!cwd.join("test_build_bad.kpar").exists());

    Ok(())
}

#[test]
fn project_build_warns_when_file_symbol_is_missing_from_index()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "test-build-missing-index-entry",
        "1.2.3",
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package Present;\n")?;
    let out = run_sysand_in(
        &cwd,
        [
            "include",
            "--compute-checksum",
            "--no-index-symbols",
            "test.sysml",
        ],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar", "--keep-index"], None)?;
    out.assert().success().stderr(predicate::str::contains(
        "index is missing symbol `Present` found in file `test.sysml`",
    ));
    assert!(cwd.join("test-build.kpar").exists());

    Ok(())
}

/// Build a project that has a path (`file:`) usage
#[test]
fn project_build_path_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1) = cli_init_json_project(None, "acme", "test-build1", "1.2.3", None)?;
    let (_temp_dir2, cwd2) = cli_init_json_project(None, "acme", "test-build2", "1.2.3", None)?;
    let out = run_sysand_in(&cwd1, ["add", "--iri-path", cwd2.as_str()], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd1, ["build", "./test-build.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("project includes a path usage"));
    assert!(!cwd1.join("test-build.kpar").is_file());

    let out = run_sysand_in(
        &cwd1,
        ["build", "./test-build.kpar", "--allow-path-usage"],
        None,
    )?;

    // Warning must still be produced
    out.assert()
        .success()
        .stderr(predicate::str::contains("project includes a path usage"));

    let out = run_sysand_in(
        &cwd1,
        ["info", "--kpar-path", cwd1.join("test-build.kpar").as_str()],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: test-build"))
        .stdout(predicate::str::contains("Version: 1.2.3"))
        .stdout(predicate::str::contains(file_url_from_path(&cwd2)));

    let kpar_project = LocalKParProjectRaw::new_guess_root(cwd1.join("test-build.kpar"))?;

    let (Some(_), Some(_)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    Ok(())
}

/// An index usage is portable: it needs no `--allow-path-usage`, and the
/// KPAR keeps it as it is
#[test]
fn project_build_index_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-build", "1.2.3", None)?;
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "Acme Labs/My Lib",
            "--version-constraint",
            "^1",
        ],
        None,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("path usage").not());

    let info = read_kpar_file(&cwd.join("test-build.kpar"), ".project.json");
    assert!(
        info.contains(
            r#"{
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^1"
    }"#
        ),
        "{info}"
    );

    Ok(())
}

struct WProject {
    name: String,
    info_path: Utf8PathBuf,
    meta_path: Utf8PathBuf,
    orig_info_contents: String,
    orig_meta_contents: String,
}
impl WProject {
    fn new(cwd: Utf8PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let name = cwd.file_name().unwrap().to_owned();
        let info_path = cwd.join(".project.json");
        let meta_path = cwd.join(".meta.json");
        let orig_info_contents = fs::read_to_string(&info_path)?;
        let orig_meta_contents = fs::read_to_string(&meta_path)?;
        Ok(Self {
            name,
            info_path,
            meta_path,
            orig_info_contents,
            orig_meta_contents,
        })
    }
}

#[test]
/// Legacy json fixture, like the rest of this file: it pins the
/// `.workspace.json` workspace build.
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
        json!({"projects": [
        {"path": "subgroup/project1", "iris": ["urn:kpar:project1"]},
        {"path": "subgroup/project2", "iris": ["urn:kpar:project2"]},
        {"path": "project3", "iris": ["urn:kpar:project3"]}
        ]})
        .to_string(),
    )?;

    for project_cwd in [&project1_cwd, &project2_cwd, &project3_cwd] {
        std::fs::create_dir(project_cwd)?;
        write_legacy_project(
            project_cwd,
            "acme",
            project_cwd.file_name().unwrap(),
            "1.2.3",
            None,
        )?;

        std::fs::write(project_cwd.join("test.sysml"), b"package P;\n")?;
        let out = run_sysand_in(
            project_cwd,
            ["include", "--no-index-symbols", "test.sysml"],
            None,
        )?;
        out.assert().success();
    }

    let projects = [
        WProject::new(project1_cwd)?,
        WProject::new(project2_cwd)?,
        WProject::new(project3_cwd)?,
    ];

    let expected_checksum = expected_checksum();

    let out = run_sysand_in(&cwd, ["build", "--keep-index"], None)?;
    out.assert().success();

    for project in &projects {
        let kpar_path = cwd
            .join("output")
            .join(format!("{}-1.2.3.kpar", project.name));
        assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

        let out = run_sysand_in(&cwd, ["info", "--kpar-path", kpar_path.as_str()], None)?;

        out.assert()
            .success()
            .stdout(predicate::str::contains(format!("Name: {}", project.name)))
            .stdout(predicate::str::contains("Version: 1.2.3"));

        let kpar_project = LocalKParProjectRaw::new_guess_root(kpar_path)?;

        let (Some(info), Some(meta)) = kpar_project.get_project()? else {
            panic!("failed to get built project info/meta");
        };

        // Ensure the build artifact matches original project
        let original_meta: InterchangeProjectMetadataRaw =
            serde_json::from_str(&project.orig_meta_contents)?;
        let original_info: InterchangeProjectInfoRaw =
            serde_json::from_str(&project.orig_info_contents)?;
        assert_eq!(original_info, info);
        // Index should not change
        assert_eq!(meta.index, original_meta.index);
        // File hash should still be updated
        assert_eq!(meta.checksum, expected_checksum);

        // Ensure no changes were made to the original project
        let new_info_contents = fs::read_to_string(&project.info_path)?;
        let new_meta_contents = fs::read_to_string(&project.meta_path)?;
        assert_eq!(project.orig_info_contents, new_info_contents);
        assert_eq!(project.orig_meta_contents, new_meta_contents);
    }

    // Now canonicalize meta during build. Previous artifacts should be overwritten
    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success();
    let expected_index = expected_index();

    for project in &projects {
        let kpar_path = cwd
            .join("output")
            .join(format!("{}-1.2.3.kpar", project.name));
        assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

        let out = run_sysand_in(&cwd, ["info", "--kpar-path", kpar_path.as_str()], None)?;

        out.assert()
            .success()
            .stdout(predicate::str::contains(format!("Name: {}", project.name)))
            .stdout(predicate::str::contains("Version: 1.2.3"));

        let kpar_project = LocalKParProjectRaw::new_guess_root(kpar_path)?;

        let (Some(_), Some(meta)) = kpar_project.get_project()? else {
            panic!("failed to get built project info/meta");
        };

        // File hash should be updated
        assert_eq!(meta.checksum, expected_checksum);

        assert_eq!(meta.index, expected_index);

        // Ensure no changes were made to the original project
        let new_info_contents = fs::read_to_string(&project.info_path)?;
        let new_meta_contents = fs::read_to_string(&project.meta_path)?;
        assert_eq!(project.orig_info_contents, new_info_contents);
        assert_eq!(project.orig_meta_contents, new_meta_contents);
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
    write_legacy_project(&project1_cwd, "acme", "project1", "1.0.0", None)?;

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
    assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

    let kpar_project = LocalKParProjectRaw::new_guess_root(kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );

    Ok(())
}

/// Workspace with unknown `meta.metamodel` — build succeeds with a warning.
/// The value is outside both recognized metamodel families: an OMG release
/// Sysand has not heard of is a perfectly good metamodel and does not warn.
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
                "metamodel": "https://example.com/spec/SysML/20251201"
            }
        }"#,
    )?;

    std::fs::create_dir(&project1_cwd)?;
    write_legacy_project(&project1_cwd, "acme", "project1", "1.0.0", None)?;

    std::fs::write(project1_cwd.join("test.sysml"), b"package P;\n")?;
    let out = run_sysand_in(
        &project1_cwd,
        ["include", "--no-index-symbols", "test.sysml"],
        None,
    )?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build"], None)?;
    out.assert().success().stderr(predicate::str::contains(
        "`.workspace.json` declares metamodel `https://example.com/spec/SysML/20251201`, \
         which this version of Sysand does not recognize",
    ));

    let kpar_path = cwd.join("output").join("project1-1.0.0.kpar");
    assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

    let kpar_project = LocalKParProjectRaw::new_guess_root(kpar_path)?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://example.com/spec/SysML/20251201")
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
    write_legacy_project(&project1_cwd, "acme", "project1", "1.0.0", None)?;

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
        serde_json::Value::String("https://www.omg.org/spec/KerML/20250201".to_owned());
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
    write_legacy_project(&project1_cwd, "acme", "project1", "1.0.0", None)?;

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
        serde_json::Value::String("https://www.omg.org/spec/SysML/20250201".to_owned());
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
    write_legacy_project(&project1_cwd, "acme", "project1", "1.0.0", None)?;

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

    let kpar_project = LocalKParProjectRaw::new_guess_root(&kpar_path)?;
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

    let kpar_project = LocalKParProjectRaw::new_guess_root(&kpar_path)?;
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
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-readme", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::write(cwd.join("README.md"), b"# My Project\nHello world\n")?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including readme from"));

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "README.md",
        "# My Project\nHello world\n",
    );

    Ok(())
}

/// Build a project without any README file — should succeed
#[test]
fn project_build_without_readme() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-no-readme", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_missing(&cwd.join("test-build.kpar"), "README.md");

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
        write_legacy_project(project_cwd, "acme", project_name, "1.2.3", None)?;

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
            .join(format!("{project_name}-1.2.3.kpar"));
        assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

        assert_kpar_file(&kpar_path, "README.md", expected_readme);
    }

    Ok(())
}

/// Build a project with a CHANGELOG.md at the project root
#[test]
fn project_build_with_changelog() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-changelog", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::write(
        cwd.join("CHANGELOG.md"),
        b"# Changelog\n\n## 1.2.3\n- Initial release\n",
    )?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including changelog from"));

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "CHANGELOG.md",
        "# Changelog\n\n## 1.2.3\n- Initial release\n",
    );

    Ok(())
}

/// Build a project without any CHANGELOG file — should succeed
#[test]
fn project_build_without_changelog() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-no-changelog", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_missing(&cwd.join("test-build.kpar"), "CHANGELOG.md");

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
        write_legacy_project(project_cwd, "acme", project_name, "1.2.3", None)?;

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
            .join(format!("{project_name}-1.2.3.kpar"));
        assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

        assert_kpar_file(&kpar_path, "CHANGELOG.md", expected_changelog);
    }

    Ok(())
}

/// Build a project with a single SPDX license — the matching
/// `LICENSES/<id>.txt` is included in the KPAR.
#[test]
fn project_build_with_single_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "test-license", "1.2.3", Some("MIT"))?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("MIT.txt"), b"MIT license text\n")?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Including license from"));

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/MIT.txt",
        "MIT license text\n",
    );

    Ok(())
}

/// Build a project with a compound SPDX expression — both license files
/// are included.
#[test]
fn project_build_with_compound_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "test-compound-license",
        "1.2.3",
        Some("MIT OR Apache-2.0"),
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("MIT.txt"), b"MIT body\n")?;
    std::fs::write(
        cwd.join("LICENSES").join("Apache-2.0.txt"),
        b"Apache body\n",
    )?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/MIT.txt",
        "MIT body\n",
    );
    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/Apache-2.0.txt",
        "Apache body\n",
    );

    Ok(())
}

/// Build a project with a `WITH` exception — both the license and the
/// exception file are included.
#[test]
fn project_build_with_license_exception() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "test-license-with",
        "1.2.3",
        Some("GPL-2.0-only WITH Classpath-exception-2.0"),
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(cwd.join("LICENSES").join("GPL-2.0-only.txt"), b"GPL body\n")?;
    std::fs::write(
        cwd.join("LICENSES").join("Classpath-exception-2.0.txt"),
        b"Classpath exception body\n",
    )?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/GPL-2.0-only.txt",
        "GPL body\n",
    );
    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/Classpath-exception-2.0.txt",
        "Classpath exception body\n",
    );

    Ok(())
}

/// Build a project with a custom `LicenseRef-` identifier — the matching
/// file is bundled verbatim.
#[test]
fn project_build_with_license_ref() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "test-license-ref",
        "1.2.3",
        Some("LicenseRef-MyCustom"),
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("LICENSES"))?;
    std::fs::write(
        cwd.join("LICENSES").join("LicenseRef-MyCustom.txt"),
        b"Custom license body\n",
    )?;
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_file(
        &cwd.join("test-build.kpar"),
        "LICENSES/LicenseRef-MyCustom.txt",
        "Custom license body\n",
    );

    Ok(())
}

/// Build a project that declares a license but ships no matching file —
/// the build succeeds and warns about the missing license file.
#[test]
fn project_build_with_license_file_missing() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "test-missing-license", "1.2.3", Some("MIT"))?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("LICENSES/MIT.txt"));

    assert_kpar_missing(&cwd.join("test-build.kpar"), "LICENSES/MIT.txt");

    Ok(())
}

/// Build a project without any license — no LICENSES/* entries are added.
#[test]
fn project_build_without_license() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-no-license", "1.2.3", None)?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?;
    out.assert().success();

    assert_kpar_no_licenses_dir(&cwd.join("test-build.kpar"));

    Ok(())
}

fn read_kpar_file(kpar_path: &camino::Utf8Path, archive_path: &str) -> String {
    let file = std::fs::File::open(kpar_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut entry = archive
        .by_name(archive_path)
        .unwrap_or_else(|_| panic!("expected {archive_path} in {kpar_path}"));
    let mut content = String::new();
    entry.read_to_string(&mut content).unwrap();
    content
}

fn assert_kpar_file(kpar_path: &camino::Utf8Path, archive_path: &str, expected: &str) {
    let content = read_kpar_file(kpar_path, archive_path);
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
        let name = name.unwrap();
        assert!(
            !name.starts_with("LICENSES/"),
            "KPAR should not contain any LICENSES/ entries but found {name}: {kpar_path}"
        );
    }
}

fn compression_method(compression: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "test-build", "1.2.3", None)?;

    {
        let mut sysml_file = std::fs::File::create(cwd.join("test.sysml"))?;
        sysml_file.write_all(b"package P;\n")?;
    }
    let out = run_sysand_in(&cwd, ["include", "--no-index-symbols", "test.sysml"], None)?;

    out.assert().success();

    let out = match compression {
        Some(compression) => run_sysand_in(
            &cwd,
            ["build", "--compression", compression, "./test-build.kpar"],
            None,
        )?,
        None => run_sysand_in(&cwd, ["build", "./test-build.kpar"], None)?,
    };

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["info", "--kpar-path", cwd.join("test-build.kpar").as_str()],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: test-build"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let kpar_project = LocalKParProjectRaw::new_guess_root(cwd.join("test-build.kpar"))?;

    let (Some(info), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };

    assert_eq!(info.name, "test-build");
    assert_eq!(info.version, "1.2.3");

    assert_eq!(meta.checksum, expected_checksum());
    assert_eq!(meta.index, expected_index());
    let mut src = String::new();
    kpar_project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package P;\n");
    Ok(())
}

// ------------------------------------- building a `model.toml` project

/// `sysand build` on a project `sysand init` produces.
#[test]
fn build_toml_project() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "--version",
            "1.0.0",
        ],
        None,
    )?
    .assert()
    .success();
    std::fs::write(cwd.join("a.sysml"), b"package A;\n")?;
    run_sysand_in(&cwd, ["include", "a.sysml"], None)?
        .assert()
        .success();

    run_sysand_in(&cwd, ["build"], None)?.assert().success();
    assert!(
        cwd.join("output/widgets-1.0.0.kpar").exists(),
        "the archive is named `<name>-<version>.kpar`: {:?}",
        std::fs::read_dir(cwd.join("output"))?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<Vec<_>, _>>()?
    );
    Ok(())
}

/// The usage becomes a PURL constrained to the target's version.
///
/// The manifest is written by hand: what is under test here is the
/// build, not the write surface.
#[test]
fn build_toml_project_with_a_dir_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n",
    )?;
    std::fs::write(cwd.join("a.sysml"), b"package A;\n")?;
    std::fs::create_dir_all(cwd.join("dep"))?;
    write_legacy_project(&cwd.join("dep"), "acme", "dep", "1.2.3", None)?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();
    assert!(cwd.join("output/widgets-1.0.0.kpar").exists());
    Ok(())
}

/// A usage whose target defines no project, through the CLI.
#[test]
fn build_toml_project_with_a_missing_target() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n",
    )?;
    std::fs::write(cwd.join("a.sysml"), b"package A;\n")?;

    run_sysand_in(&cwd, ["build"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "usage `acme/dep` targets `dep`, which defines no project",
        ));

    // The flag keeps it a path usage, so no target is read at all.
    run_sysand_in(&cwd, ["build", "--allow-path-usage"], None)?
        .assert()
        .success();
    Ok(())
}

/// A standard-library usage becomes the IRI Sysand embeds.
#[test]
fn build_toml_project_with_a_stdlib_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\n\n\
         [[inline-usages]]\npublisher = \"OMG\"\nname = \"Kernel Semantic Library\"\n\
         version = \"^1.0.0\"\n",
    )?;
    std::fs::write(cwd.join("a.sysml"), b"package A;\n")?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();

    let archive = std::fs::File::open(cwd.join("output/widgets-1.0.0.kpar"))?;
    let mut zip = zip::ZipArchive::new(archive)?;
    let mut info = String::new();
    std::io::Read::read_to_string(&mut zip.by_name(".project.json")?, &mut info)?;
    assert!(
        info.contains("https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"),
        "{info}"
    );
    Ok(())
}

/// The `Building kpar` lines appear in `members` order, whatever the
/// members' `dir` usages of each other.
///
/// `contains` cannot express ordering, so the assertion is on the
/// relative byte offsets of the three lines in stderr.
#[test]
fn workspace_build_follows_members_order() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [{"path": "gamma", "iris": []}, {"path": "beta", "iris": []}, {"path": "alpha", "iris": []}]}"#,
    )?;
    for (at, dep) in [
        ("alpha", None),
        ("beta", Some("../alpha")),
        ("gamma", Some("../beta")),
    ] {
        let path = cwd.join(at);
        std::fs::create_dir_all(&path)?;
        let usage = dep.map_or_else(String::new, |d| {
            format!("\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"{d}\"\n")
        });
        std::fs::write(
            path.join("model.toml"),
            format!(
                "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"{at}\"\n\
                 version = \"1.0.0\"\nsources = [\"m.kerml\"]\n{usage}"
            ),
        )?;
        std::fs::write(path.join("m.kerml"), b"package M;\n")?;
    }

    let out = run_sysand_in(&cwd, ["build", "--allow-path-usage"], None)?;
    out.clone().assert().success();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let at = |name: &str| {
        stderr
            .find(&format!("{name}-1.0.0.kpar"))
            .unwrap_or_else(|| panic!("`{name}` was not built:\n{stderr}"))
    };
    assert!(
        at("gamma") < at("beta") && at("beta") < at("alpha"),
        "members must be built in `members` order:\n{stderr}"
    );
    Ok(())
}

/// Every failure is reported, and an independent member is
/// still built.
///
/// The failures must be distinguishable, and their error texts are
/// identical -- so the assertion is on the member names the CLI adds.
#[test]
fn workspace_build_reports_every_failure() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [{"path": "alpha", "iris": []}, {"path": "beta", "iris": []}, {"path": "gamma", "iris": []}]}"#,
    )?;
    std::fs::create_dir_all(cwd.join("alpha"))?;
    std::fs::create_dir_all(cwd.join("beta"))?;
    let good = cwd.join("gamma");
    std::fs::create_dir_all(&good)?;
    write_legacy_project(&good, "acme", "gamma", "1.0.0", None)?;

    run_sysand_in(&cwd, ["build"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to build member `alpha`"))
        .stderr(predicate::str::contains("failed to build member `beta`"))
        .stderr(predicate::str::contains(
            "2 of 3 workspace members failed to build",
        ));
    assert!(
        cwd.join("output/gamma-1.0.0.kpar").exists(),
        "an independent member is still built"
    );
    Ok(())
}

/// A build never reads another member's archive, so a member whose
/// dependency failed is built all the same.
#[test]
fn workspace_build_builds_a_dependent_of_a_failed_member() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [{"path": "beta", "iris": []}, {"path": "alpha", "iris": []}]}"#,
    )?;
    std::fs::create_dir_all(cwd.join("alpha"))?;
    let beta = cwd.join("beta");
    std::fs::create_dir_all(&beta)?;
    std::fs::write(
        beta.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"beta\"\n\
         version = \"1.0.0\"\nsources = [\"m.kerml\"]\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"alpha\"\ndir = \"../alpha\"\n",
    )?;
    std::fs::write(beta.join("m.kerml"), b"package M;\n")?;

    run_sysand_in(&cwd, ["build", "--allow-path-usage"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to build member `alpha`"))
        .stderr(predicate::str::contains(
            "1 of 2 workspace members failed to build",
        ));
    assert!(
        cwd.join("output/beta-1.0.0.kpar").exists(),
        "the dependent is built"
    );
    Ok(())
}
