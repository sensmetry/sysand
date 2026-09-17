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

/// A `model.toml` workspace root listing `members`.
fn write_workspace(
    cwd: &camino::Utf8Path,
    members: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    write_workspace_with(cwd, members, "")
}

/// As [`write_workspace`], with extra `[workspace]` keys -- a
/// `metamodel` for members that inherit one.
fn write_workspace_with(
    cwd: &camino::Utf8Path,
    members: &[&str],
    extra: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
    std::fs::write(
        cwd.join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [{}]\n{extra}",
            entries.join(", ")
        ),
    )?;
    Ok(())
}

/// A `model.toml` member with one source file, `test.sysml`, which is
/// what [`expected_checksum`] and [`expected_index`] describe.
///
/// No `license`: a build demands a matching `LICENSES/` file for one,
/// and none of these fixtures is about licences.
fn write_toml_member(
    dir: &camino::Utf8Path,
    name: &str,
    version: &str,
    extra: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    // Every project declares a metamodel; a test that is about which one
    // writes it into `extra` instead.
    let metamodel = if extra.contains("metamodel") {
        ""
    } else {
        "metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
    };
    std::fs::write(
        dir.join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
             version = \"{version}\"\nsources = [\"test.sysml\"]\n{metamodel}{extra}"
        ),
    )?;
    std::fs::write(dir.join("test.sysml"), b"package P;\n")?;
    Ok(())
}

/// A workspace build writes one archive per member under the workspace
/// root's `output/`, whatever depth the member sits at, and leaves the
/// members themselves untouched.
#[test]
fn workspace_build() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_workspace(
        &cwd,
        &["subgroup/project1", "subgroup/project2", "project3"],
    )?;

    let members = [
        cwd.join("subgroup/project1"),
        cwd.join("subgroup/project2"),
        cwd.join("project3"),
    ];
    for member in &members {
        write_toml_member(member, member.file_name().unwrap(), "1.2.3", "")?;
    }
    let before = members
        .iter()
        .map(|m| std::fs::read_to_string(m.join("model.toml")))
        .collect::<Result<Vec<_>, _>>()?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();

    for member in &members {
        let name = member.file_name().unwrap();
        let kpar_path = cwd.join("output").join(format!("{name}-1.2.3.kpar"));
        assert!(kpar_path.is_file(), "kpar file does not exist: {kpar_path}");

        let out = run_sysand_in(&cwd, ["info", "--kpar-path", kpar_path.as_str()], None)?;

        out.assert()
            .success()
            .stdout(predicate::str::contains(format!("Name: {name}")))
            .stdout(predicate::str::contains("Version: 1.2.3"));

        let kpar_project = LocalKParProjectRaw::new_guess_root(kpar_path)?;
        let (Some(info), Some(meta)) = kpar_project.get_project()? else {
            panic!("failed to get built project info/meta");
        };
        assert_eq!(info.name, name);
        assert_eq!(info.version, "1.2.3");
        assert_eq!(meta.checksum, expected_checksum());
        assert_eq!(meta.index, expected_index());
    }

    // The build reads the members; it must not write to them.
    for (member, text) in members.iter().zip(&before) {
        assert_eq!(&std::fs::read_to_string(member.join("model.toml"))?, text);
    }

    Ok(())
}

/// Explicit inheritance: a workspace `metamodel` reaches a member that
/// asks for it with `metamodel.workspace = true`, and only then.
/// `workspaces.md` has no workspace-level metamodel, so the member has
/// to say so.
#[test]
fn workspace_build_with_inherited_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_workspace_with(
        &cwd,
        &["project1"],
        "metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;
    write_toml_member(
        &cwd.join("project1"),
        "project1",
        "1.0.0",
        "metamodel.workspace = true\n",
    )?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();

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

/// A member that does *not* ask to inherit declares its own, and the
/// build carries that one rather than the workspace's.
///
/// The counterpart of the test above, and the whole difference between
/// explicit inheritance and injecting the workspace's value. Every
/// project declares a metamodel (model-toml.md "Metamodel"), so what
/// this proves is that the workspace's value does not leak into a
/// member that did not ask.
#[test]
fn a_member_that_does_not_inherit_declares_its_own() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_workspace_with(
        &cwd,
        &["project1"],
        "metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;
    write_toml_member(
        &cwd.join("project1"),
        "project1",
        "1.0.0",
        "metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n",
    )?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();

    let kpar_project =
        LocalKParProjectRaw::new_guess_root(cwd.join("output").join("project1-1.0.0.kpar"))?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.sysand.com/spec/SysML/20250201")
    );

    Ok(())
}

/// An unrecognized `[workspace] metamodel` is refused where it is
/// written: a group's value is held to the four, since the only thing it
/// is for is being inherited by a `[project]`.
#[test]
fn an_unrecognized_workspace_metamodel_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_workspace_with(
        &cwd,
        &["project1"],
        "metamodel = \"https://example.com/spec/SysML/20251201\"\n",
    )?;
    write_toml_member(
        &cwd.join("project1"),
        "project1",
        "1.0.0",
        "metamodel.workspace = true\n",
    )?;

    run_sysand_in(&cwd, ["build"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "`metamodel` in `[workspace]` is `https://example.com/spec/SysML/20251201`, \
             which is not one of the four values a `model.toml` recognizes",
        ));
    assert!(!cwd.join("output").exists(), "nothing was built");

    Ok(())
}

/// A member that declares its own metamodel builds with it, beside a
/// workspace that declares another.
///
/// The specification gives the two nothing to conflict over: "There is
/// no workspace-level metamodel. Members with different metamodels ...
/// may share a workspace." So the question is whether the member's own
/// value reaches the archive.
#[test]
fn a_member_declares_its_own_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_workspace_with(
        &cwd,
        &["project1"],
        "metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;
    write_toml_member(
        &cwd.join("project1"),
        "project1",
        "1.0.0",
        "metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n",
    )?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();

    let kpar_project =
        LocalKParProjectRaw::new_guess_root(cwd.join("output").join("project1-1.0.0.kpar"))?;
    let (Some(_), Some(meta)) = kpar_project.get_project()? else {
        panic!("failed to get built project info/meta");
    };
    assert_eq!(
        meta.metamodel.as_deref(),
        Some("https://www.sysand.com/spec/SysML/20250201"),
        "the member's own metamodel, not the workspace's"
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

    write_workspace(&cwd, &["project1", "project2"])?;

    for (project_cwd, readme_content) in [
        (&project1_cwd, "# Project 1\n"),
        (&project2_cwd, "# Project 2\n"),
    ] {
        write_toml_member(project_cwd, project_cwd.file_name().unwrap(), "1.2.3", "")?;
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

    write_workspace(&cwd, &["project1", "project2"])?;

    for (project_cwd, changelog_content) in [
        (&project1_cwd, "# Project 1 Changelog\n"),
        (&project2_cwd, "# Project 2 Changelog\n"),
    ] {
        write_toml_member(project_cwd, project_cwd.file_name().unwrap(), "1.2.3", "")?;
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
            "--metamodel",
            SYSML,
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
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
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
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
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

/// A standard-library usage is a publisher-less `kpar-url`, and
/// reaches the archive as the IRI it already names.
#[test]
fn build_toml_project_with_a_stdlib_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"a.sysml\"]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
         [[inline-usages]]\nname = \"Kernel Semantic Library\"\n\
         kpar-url = \"https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar\"\n",
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
    write_workspace(&cwd, &["gamma", "beta", "alpha"])?;
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
                "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{at}\"\n\
                 version = \"1.0.0\"\nsources = [\"m.kerml\"]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n{usage}"
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

/// A member that loads and cannot be built: its manifest is valid, so
/// the workspace loads, and the source it lists is not there.
///
/// Not an empty directory: a workspace
/// member must define a project, and `Workspace::load_with` refuses one
/// that does not -- which fails the whole load and never reaches the
/// per-member reporting these tests are about.
fn write_broken_member(
    dir: &camino::Utf8Path,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
             version = \"1.0.0\"\nsources = [\"missing.sysml\"]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
        ),
    )?;
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
    write_workspace(&cwd, &["alpha", "beta", "gamma"])?;
    write_broken_member(&cwd.join("alpha"), "alpha")?;
    write_broken_member(&cwd.join("beta"), "beta")?;
    write_toml_member(&cwd.join("gamma"), "gamma", "1.0.0", "")?;

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
    write_workspace(&cwd, &["beta", "alpha"])?;
    write_broken_member(&cwd.join("alpha"), "alpha")?;
    write_toml_member(
        &cwd.join("beta"),
        "beta",
        "1.0.0",
        "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"alpha\"\ndir = \"../alpha\"\n",
    )?;

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

// ------------------- `build` reads the lockfile of its scope

/// `build` reads the lockfile of its scope when one exists and validates
/// it before building. Parsing is the validation, so a lockfile
/// carrying a metamodel direction error stops the build.
#[test]
fn build_rejects_a_lockfile_with_a_direction_error() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_toml_member(&cwd, "widgets", "1.0.0", "")?;
    std::fs::write(
        cwd.join("sysand-lock.toml"),
        "lock_version = \"0.5\"\n\
         \n[[project]]\npublisher = \"acme\"\nname = \"app\"\nversion = \"1.0.0\"\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\
         identifiers = [\"urn:app\"]\nusages = [\"urn:lib\"]\n\
         \n[[project]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"1.0.0\"\n\
         metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n\
         identifiers = [\"urn:lib\"]\n",
    )?;

    run_sysand_in(&cwd, ["build"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "only Syside-metamodel projects may depend on Syside-metamodel projects",
        ));
    assert!(
        !cwd.join("output").exists(),
        "the check runs before anything is built"
    );
    Ok(())
}

/// Without a lockfile `build` proceeds: it resolves no usages, and the
/// edge checks run at the next `lock`.
#[test]
fn build_without_a_lockfile_proceeds() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_toml_member(&cwd, "widgets", "1.0.0", "")?;

    run_sysand_in(&cwd, ["build"], None)?.assert().success();
    assert!(cwd.join("output/widgets-1.0.0.kpar").is_file());
    Ok(())
}
