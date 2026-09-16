// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Commands run *inside* a member whose manifest inherits.
//!
//! `model-toml.md` "Inheritance": a marker resolves against the
//! workspace table, and only for a member. So a member with
//! `version.workspace = true` must be readable by `info`, `sources` and
//! `build` run from inside it.

use assert_cmd::prelude::*;
use camino::Utf8PathBuf;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// Inherits both keys, and has a real source file: a member with
/// `sources = []` prints nothing, so a `sources` assertion over it
/// would hold no matter what the command did.
const INHERITING: &str = "\
manifest-version = \"1.0.0\"

[project]
publisher.workspace = true
name = \"app\"
version.workspace = true
sources = [\"src/main.sysml\"]
metamodel.workspace = true
";

const ROOT: &str = "\
manifest-version = \"1.0.0\"

[workspace]
members = [\"app\"]
exclude = [\"vendor\"]
publisher = \"acme\"
version = \"1.2.3\"
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

/// A workspace with one inheriting member `app`, and an *excluded*
/// project `vendor/dep` whose manifest is byte-for-byte `app`'s.
///
/// Excluded, not unlisted. An unlisted project never reaches the member
/// gate -- `start_region` refuses it and `run_cli` propagates -- so a
/// negative test built on one passes with the gate deleted.
fn workspace() -> Result<(camino_tempfile::Utf8TempDir, Utf8PathBuf), Box<dyn std::error::Error>> {
    let (dir, root) = new_temp_cwd()?;
    std::fs::write(root.join("model.toml"), ROOT)?;
    for project in ["app", "vendor/dep"] {
        let at = root.join(project);
        std::fs::create_dir_all(at.join("src"))?;
        std::fs::write(at.join("model.toml"), INHERITING)?;
        std::fs::write(at.join("src/main.sysml"), "package Main;\n")?;
    }
    Ok((dir, root))
}

/// The `--get` route, which reads the current project.
#[test]
fn member_info_version_reads_an_inherited_value() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp, root) = workspace()?;
    run_sysand_in(&root.join("app"), ["info", "--get", "version"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("1.2.3"));
    Ok(())
}

/// The bare `info` route, which must report the current project rather
/// than re-open its path with no table.
#[test]
fn member_info_reads_an_inherited_value() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp, root) = workspace()?;
    run_sysand_in(&root.join("app"), ["info"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("acme"));
    Ok(())
}

#[test]
fn member_sources_lists_its_own_file() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp, root) = workspace()?;
    run_sysand_in(&root.join("app"), ["sources"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("main.sysml"));
    Ok(())
}

/// No `dir` usage to another member: a member that depends on an
/// *inheriting* sibling by path is not covered here.
#[test]
fn member_build_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp, root) = workspace()?;
    run_sysand_in(&root.join("app"), ["build"], None)?
        .assert()
        .success();
    Ok(())
}

/// The negative, and the only test here that distinguishes the gate
/// from "any project under a workspace root".
///
/// `vendor/dep` is under `exclude`, so `start_region` answers
/// `Standalone` and `member_index` answers `None`. Its manifest is the
/// member's, so nothing but its location decides the outcome.
#[test]
fn an_excluded_project_refuses_to_inherit() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp, root) = workspace()?;
    run_sysand_in(&root.join("vendor/dep"), ["info", "--get", "version"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the project is not a member of any workspace",
        ));
    Ok(())
}
