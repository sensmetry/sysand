// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The standard-library workspace fixture.
//!
//! These tests read the fixture and nothing else, so they run without
//! the release sources. `Workspace::load_with` resolves every member's
//! manifest and never opens a `sources` entry -- source existence is
//! checked only by a build or a publish.
#![cfg(feature = "filesystem")]

use camino::Utf8PathBuf;
use sysand_core::workspace::model_toml::{MemberChecks, Workspace};

fn fixture() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/stdlib-workspace")
}

fn load() -> Workspace {
    Workspace::load_with(fixture(), MemberChecks::All)
        .unwrap_or_else(|error| panic!("the standard-library fixture loads:\n{error}"))
}

#[test]
fn the_workspace_has_ten_members() {
    assert_eq!(load().members().len(), 10);
}

/// Every member inherits its metamodel from the `kerml` or `sysml`
/// group, and the value that arrives is the release's.
#[test]
fn every_member_inherits_its_metamodel() {
    for member in load().members() {
        let metamodel = member
            .project
            .metamodel
            .as_deref()
            .unwrap_or_else(|| panic!("`{}` resolves a metamodel", member.path.raw));
        assert!(
            metamodel == "https://www.omg.org/spec/KerML/20250201"
                || metamodel == "https://www.omg.org/spec/SysML/20250201",
            "`{}` has `{metamodel}`",
            member.path.raw
        );
    }
}

/// Every member carries `stdlib`, which is what makes the Kernel cycle
/// legal.
#[test]
fn every_member_is_a_standard_library() {
    for member in load().members() {
        assert!(
            member.project.stdlib.is_some(),
            "`{}` carries `stdlib`",
            member.path.raw
        );
    }
}

/// Semantic, Data-Type and Function are mutually dependent, and the
/// release ships them that way. Every project on the cycle carries
/// `stdlib`, so the exception in model-toml.md "Circular dependencies"
/// applies.
#[test]
fn the_kernel_cycle_is_accepted() {
    let workspace = load();
    let out = camino_tempfile::tempdir().expect("a temporary directory");

    // A build reads every source file, which this fixture does not
    // carry -- so the members fail individually.
    let failure = sysand_core::commands::build::do_build_workspace_kpars(
        &workspace,
        out.path(),
        sysand_core::build::KparCompressionMethod::default(),
        true,
        false,
    )
    .expect_err("the fixture carries no sources");

    assert_eq!(
        failure.failed.len() + failure.built.len(),
        10,
        "every member is attempted"
    );
}
