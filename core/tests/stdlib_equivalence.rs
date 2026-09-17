// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `model-toml.md` "Standard library" as an executable test: what
//! `sysand build` produces from the workspace fixture must be the
//! release OMG published.
//!
//! **This test does not run without the release sources.** They are
//! deliberately not in this repository; `SYSAND_STDLIB_SOURCES` must
//! point at a checkout of `Systems-Modeling/SysML-v2-Release`, which CI
//! provides on one Linux matrix entry. Without it the test prints a
//! notice and returns -- a skip that says so, rather than a pass that
//! does not.
//!
//! **Which checkout: tag `2025-09`, not `2025-02`.** The release
//! repository tags by month, so the tag that looks like this spec
//! version is `2025-02` -- but the test cannot pass there: the library
//! sources kept changing after February, and 16 of the 58 files that
//! `stdlib_assets/20250201` carries a checksum for differ at `2025-02`
//! (all ten of Systems Library's changed files, plus Analysis,
//! Geometry, Quantities and Units, and Requirement Derivation). At
//! `2025-09` all 58 match and this test passes.
//!
//! That is a provenance question this repository cannot answer: the
//! OMG artifacts these assets describe live at
//! `https://www.omg.org/spec/.../20250201/*.kpar`, which is not
//! reachable from the development environment, so whether OMG
//! re-published those KPARs against the maintained library or whether
//! the assets were harvested from a later checkout is unverified. What
//! is verified is which tag reproduces them.
#![cfg(feature = "filesystem")]

use camino::{Utf8Path, Utf8PathBuf};
use sysand_core::{
    build::KparCompressionMethod,
    commands::build::do_build_workspace_kpars,
    stdlib_compare::{
        compare_to_published, published_sources, release_paths, stage_release_workspace,
    },
    workspace::model_toml::{MemberChecks, Workspace},
};

const RELEASE: &str = "20250201";

fn fixture() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/stdlib-workspace")
}

fn assets() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("src/stdlib_assets/{RELEASE}"))
}

/// The fixture, with the release's sources copied beside each member's
/// `model.toml`. The layout table and the copying are library code
/// (`stdlib_compare`), because the CLI suites stage the same fixture and
/// a second copy of that table could drift from the first.
fn staged(sources_root: &Utf8Path) -> Result<camino_tempfile::Utf8TempDir, String> {
    let staging = camino_tempfile::tempdir().map_err(|e| e.to_string())?;
    stage_release_workspace(&fixture(), sources_root, staging.path())?;
    Ok(staging)
}

/// The files a library publishes, from its `.meta.json` index -- the
/// same derivation the fixture generator uses for `sources`.
fn source_names(asset: &str) -> Result<Vec<String>, String> {
    let meta = std::fs::read_to_string(assets().join(format!("{asset}.meta.json")))
        .map_err(|e| format!("reading `{asset}`'s metadata: {e}"))?;
    let meta: serde_json::Value = serde_json::from_str(&meta).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = meta
        .get("index")
        .and_then(serde_json::Value::as_object)
        .map(|index| {
            index
                .values()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

#[test]
fn the_built_workspace_equals_the_published_release() {
    // Empty counts as unset: the CI expression that selects one matrix
    // entry sets the variable to `''` everywhere else.
    let sources_root = std::env::var("SYSAND_STDLIB_SOURCES")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let Some(sources_root) = sources_root else {
        // Not `panic!`, and not silence: the notice is how an unset
        // variable in CI is visible rather than green.
        println!(
            "skipping: set SYSAND_STDLIB_SOURCES to a checkout of the \
             SysML v2 {RELEASE} release to run the equivalence test"
        );
        return;
    };
    let sources_root = Utf8PathBuf::from(sources_root);

    let staging = staged(&sources_root).expect("the fixture stages");
    let workspace = Workspace::load_with(staging.path(), MemberChecks::All)
        .unwrap_or_else(|e| panic!("the staged workspace loads:\n{e}"));
    let out = staging.path().join("output");
    std::fs::create_dir_all(&out).expect("the output directory");

    let built = do_build_workspace_kpars(
        &workspace,
        &out,
        KparCompressionMethod::default(),
        true,
        false,
    )
    .unwrap_or_else(|failure| {
        let failed: Vec<String> = failure.failed.iter().map(ToString::to_string).collect();
        panic!("every member builds:\n{}", failed.join("\n"))
    });

    let mut differences = Vec::new();
    for (asset, relative) in &release_paths() {
        let archive = built
            .iter()
            .find(|built| {
                built
                    .get_info()
                    .ok()
                    .flatten()
                    .is_some_and(|info| info.name == published_name(asset))
            })
            .unwrap_or_else(|| panic!("`{asset}` was built"));

        let names = source_names(asset).expect("the index reads");
        let sources = published_sources(&sources_root.join(relative), &names)
            .expect("the published sources read");
        let info = std::fs::read_to_string(assets().join(format!("{asset}.project.json")))
            .expect("the published info reads");
        let meta = std::fs::read_to_string(assets().join(format!("{asset}.meta.json")))
            .expect("the published metadata reads");

        for difference in compare_to_published(archive, &info, &meta, &sources) {
            differences.push(format!("{asset}: {difference}"));
        }
    }

    assert!(
        differences.is_empty(),
        "the built archives differ from the published release:\n{}",
        differences.join("\n")
    );
}

fn published_name(asset: &str) -> String {
    let info = std::fs::read_to_string(assets().join(format!("{asset}.project.json")))
        .expect("the published info reads");
    let info: serde_json::Value = serde_json::from_str(&info).expect("valid json");
    info["name"].as_str().expect("a name").to_owned()
}

use sysand_core::project::ProjectRead as _;
