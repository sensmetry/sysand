// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Tests for notices about ignored std libs

use assert_cmd::prelude::*;
use predicates::prelude::{
    predicate::str::{contains, is_empty},
    *,
};
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{self, ConfigProject, OverrideSource},
    lock::Lock,
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

const FUNCTION_LIBRARY_IRI: &str = "https://www.omg.org/spec/KerML/20250201/Function-Library.kpar";

/// `sysand add <std-iri>` (with lock) records the usage, prints a plain
/// `note:` (not a `warning:`), and produces a lockfile recording the lib as a
/// provided (source-less) dependency
#[test]
fn add_std_lib_direct_note_still_locks_skips_sync() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("u", "add_std_direct", "1.2.3")?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["add", FUNCTION_LIBRARY_IRI], None)?;

    out.assert()
        .success()
        .stderr(contains(
            "note: SysMLv2/KerML standard libraries will not be installed during sync",
        ))
        .stderr(contains("warning:").not());

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add_std_direct",
  "publisher": "u",
  "version": "1.2.3",
  "usage": [
    {{
      "resource": "{FUNCTION_LIBRARY_IRI}"
    }}
  ]
}}
"#
        )
    );

    let lock_file: Lock =
        toml::from_str(&std::fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?)?;
    let std_project = lock_file
        .projects
        .iter()
        .find(|p| p.identifiers.iter().any(|i| i == FUNCTION_LIBRARY_IRI))
        .unwrap_or_else(|| {
            panic!(
                "lockfile should still record the std lib dependency:\n{:#?}",
                lock_file.projects
            )
        });
    assert!(
        std_project.sources.is_empty(),
        "std lib entry should have no sources, since it is provided rather than installed"
    );

    Ok(())
}

/// `sysand env install <std-iri>` (with and without `--path`) warns that installing std
/// libs directly is not recommended, and does not install anything.
#[test]
fn env_install_std_lib_direct_no_path_warns_and_skips() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, _) = run_sysand(["env"], None)?;

    let out = run_sysand_in(
        &cwd,
        ["env", "install", FUNCTION_LIBRARY_IRI, "--no-index"],
        None,
    )?;

    out.assert()
        .success()
        .stderr(contains(
            "it's not recommended to install SysML v2/KerML standard",
        ))
        .stderr(contains(format!(
            "library package `{FUNCTION_LIBRARY_IRI}`,"
        )))
        .stderr(contains("to proceed anyway, pass `--include-std`"));

    let test_path = fixture_path("test_lib");
    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            FUNCTION_LIBRARY_IRI,
            "--path",
            test_path.as_str(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stderr(contains(
            "it's not recommended to install SysML v2/KerML standard",
        ))
        .stderr(contains(format!(
            "library package `{FUNCTION_LIBRARY_IRI}`,"
        )));

    run_sysand_in(&cwd, ["env", "list"], None)?
        .assert()
        .success()
        .stdout(is_empty());

    Ok(())
}

/// `sysand env install <iri>` (no `--path`), where `<iri>`'s own
/// dependencies transitively include a std lib, prints a `note:` that the
/// std lib will not be installed, distinct from the warning given when the
/// std lib itself is installed directly.
#[test]
fn env_install_transitive_std_deps_note() -> Result<(), Box<dyn std::error::Error>> {
    // A project that itself depends on a standard library.
    let (_dep_temp_dir, cwd_dep, out) =
        cli_init_project_basic("a", "env_install_std_dep", "1.2.3")?;
    out.assert().success();

    std::fs::write(cwd_dep.join("EnvInstallStdDep.sysml"), "package Dep;")?;
    run_sysand_in(&cwd_dep, ["include", "EnvInstallStdDep.sysml"], None)?
        .assert()
        .success();

    run_sysand_in(
        &cwd_dep,
        ["add", "--no-lock", "--include-std", FUNCTION_LIBRARY_IRI],
        None,
    )?
    .assert()
    .success();

    // Register the dependency project as a config override so it can be
    // resolved by IRI without an index.
    let (_temp_dir, cwd, _) = run_sysand(["env"], None)?;
    let cfg_path = cwd.join("sysand.toml");
    let cfg = toml::to_string(&config::Config {
        indexes: vec![],
        projects: vec![ConfigProject {
            identifiers: vec!["urn:kpar:env_install_std_dep".to_owned()],
            sources: vec![OverrideSource::LocalSrc {
                src_path: cwd_dep.as_str().into(),
            }],
        }],
    })?;
    std::fs::write(&cfg_path, cfg)?;

    let out = run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:env_install_std_dep",
            "--no-index",
        ],
        Some(cfg_path.as_str()),
    )?;

    out.assert()
        .success()
        .stderr(contains(
            "note: SysML v2/KerML standard library packages will not be installed during sync,",
        ))
        .stderr(contains("pass `--include-std` flag"))
        // The direct-install warning must not also fire for a transitive dep.
        .stderr(contains("it's not recommended to install").not());

    Ok(())
}

/// `sysand info` on a project whose only usage is a std lib prints the
/// "All usages are ignored" note instead of an empty/misleading usage list.
#[test]
fn info_all_usages_ignored_std_only() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "info_std_only", "1.2.3")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--include-std", FUNCTION_LIBRARY_IRI],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["info"], None)?.assert().success().stdout(
        contains(
            "All usages are ignored. Standard library usages are not\nshown by default, unless `--include-std` is passed",
        ),
    );

    // With --include-std, the usage is shown instead of being ignored.
    run_sysand_in(&cwd, ["info", "--include-std"], None)?
        .assert()
        .success()
        .stdout(contains("Usages:"))
        .stdout(contains(FUNCTION_LIBRARY_IRI))
        .stdout(contains("All usages are ignored").not());

    Ok(())
}

/// `sysand info` on a project with a mix of a normal usage and a std lib
/// usage prints the normal usage plus the "Some usages are ignored" note.
#[test]
fn info_some_usages_ignored_mixed() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "info_mixed", "1.2.3")?;
    out.assert().success();

    run_sysand_in(&cwd, ["add", "--no-lock", "urn:kpar:normal-dep"], None)?
        .assert()
        .success();
    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--include-std", FUNCTION_LIBRARY_IRI],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["info"], None)?
        .assert()
        .success()
        .stdout(contains("Usages:"))
        .stdout(contains("urn:kpar:normal-dep"))
        .stdout(contains(FUNCTION_LIBRARY_IRI).not())
        .stdout(contains(
            "Some usages are ignored. Standard library usages are not\nshown by default, unless `--include-std` is passed",
        ));

    Ok(())
}
