// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! `sysand include` and `sysand exclude`, in both manifest formats.
//!
//! The tests that create a `.project.json` pair with
//! `sysand init --kind project-json` are the coverage of the json format, and most of them assert on
//! `.meta.json`, the checksum and symbol maps a `model.toml` project does
//! not have.
//!
//! The toml tests are at the end of the file. Between them they cover
//! what `include` and `exclude` do to a manifest, and which flags they
//! refuse on one.

use std::{fs, io::Write as _};

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use predicates::prelude::*;
use sysand_core::model::{
    InterchangeProjectChecksumRaw, InterchangeProjectMetadataRaw, KerMlChecksumAlg,
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn include_and_exclude_simple() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "include-and-exclude", "1.2.3", None)?;

    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([("P".to_owned(), "test.sysml".to_owned()),]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_owned(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(meta.index, Some(IndexMap::new()));
    assert!(meta.checksum.is_none_or(|x| x.is_empty()));

    Ok(())
}

#[test]
fn include_no_checksum() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "include-no-checksum", "1.2.3", None)?;

    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([("P".to_owned(), "test.sysml".to_owned()),]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: String::new(),
                algorithm: KerMlChecksumAlg::None.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_no_index() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "include-and-exclude", "1.2.3", None)?;

    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    let out = run_sysand_in(
        &cwd,
        [
            "include",
            "test.sysml",
            "--compute-checksum",
            "--no-index-symbols",
        ],
        None,
    )?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(meta.index, Some(IndexMap::new()));

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_owned(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_empty_and_update() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "include-and-exclude", "1.2.3", None)?;

    let mut sysml_file = fs::File::create(cwd.join("test.sysml"))?;
    sysml_file.sync_all()?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(meta.index, Some(IndexMap::new()));

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_owned(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    sysml_file.write_all(b"package P;\n")?;
    sysml_file.sync_all()?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([("P".to_owned(), "test.sysml".to_owned()),]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_owned(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_same_file_replaces_old_symbols() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "include-same-file-replaces-old-symbols",
        "1.2.3",
        None,
    )?;

    fs::write(cwd.join("test.sysml"), b"package A;\npackage B;\n")?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([
            ("A".to_owned(), "test.sysml".to_owned()),
            ("B".to_owned(), "test.sysml".to_owned()),
        ]))
    );

    fs::write(cwd.join("test.sysml"), b"package C;\n")?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([("C".to_owned(), "test.sysml".to_owned()),]))
    );

    Ok(())
}

#[test]
fn include_and_exclude_both_nested() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "include-and-exclude", "1.2.3", None)?;

    fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    fs::create_dir(cwd.join("extra"))?;
    fs::write(cwd.join("extra").join("test.sysml"), b"package Extra;\n")?;

    let out = run_sysand_in(
        &cwd,
        [
            "include",
            "--compute-checksum",
            "test.sysml",
            "extra/test.sysml",
        ],
        None,
    )?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([
            ("Extra".to_owned(), "extra/test.sysml".to_owned()),
            ("P".to_owned(), "test.sysml".to_owned()),
        ]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([
            (
                "extra/test.sysml".to_owned(),
                InterchangeProjectChecksumRaw {
                    value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                        .to_owned(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
            (
                "test.sysml".to_owned(),
                InterchangeProjectChecksumRaw {
                    value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                        .to_owned(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
        ])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml", "extra/test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(meta.index, Some(IndexMap::new()));
    assert!(meta.checksum.is_none_or(|x| x.is_empty()));

    Ok(())
}

#[test]
fn include_and_exclude_single_nested() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "acme",
        "include-and-exclude-single-nested",
        "1.2.3",
        None,
    )?;

    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    fs::create_dir(cwd.join("extra"))?;

    fs::write(cwd.join("extra").join("test.sysml"), b"package Extra;\n")?;

    let out = run_sysand_in(
        &cwd,
        [
            "include",
            "--compute-checksum",
            "test.sysml",
            "extra/test.sysml",
        ],
        None,
    )?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([
            ("Extra".to_owned(), "extra/test.sysml".to_owned()),
            ("P".to_owned(), "test.sysml".to_owned()),
        ]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([
            (
                "extra/test.sysml".to_owned(),
                InterchangeProjectChecksumRaw {
                    value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                        .to_owned(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
            (
                "test.sysml".to_owned(),
                InterchangeProjectChecksumRaw {
                    value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                        .to_owned(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
        ])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        Some(IndexMap::from([(
            "Extra".to_owned(),
            "extra/test.sysml".to_owned()
        ),]))
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "extra/test.sysml".to_owned(),
            InterchangeProjectChecksumRaw {
                value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                    .to_owned(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

/// `sysand init --kind <kind>` with fixed names, in a fresh directory.
fn init_of_kind(
    kind: &str,
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--kind",
            kind,
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "nonexistent",
            "--version",
            "1.2.3",
        ],
        None,
    )?;
    out.assert().success();
    Ok((temp_dir, cwd))
}

/// The manifest files a project of either kind consists of, as written.
fn manifest_files(
    cwd: &camino::Utf8Path,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let mut files = vec![];
    for name in ["model.toml", ".project.json", ".meta.json"] {
        if exists(cwd.join(name)) {
            files.push((name.to_owned(), fs::read_to_string(cwd.join(name))?));
        }
    }
    Ok(files)
}

#[test]
fn include_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    for kind in ["project-json", "model-toml"] {
        let (_temp_dir, cwd) = init_of_kind(kind)?;
        let before = manifest_files(&cwd)?;

        let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

        out.assert().failure().stderr(predicates::str::contains(
            "`test.sysml` does not exist or is not a file",
        ));

        assert_eq!(before, manifest_files(&cwd)?, "{kind}");
        if kind == "project-json" {
            let meta: InterchangeProjectMetadataRaw =
                serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

            assert_eq!(meta.index, Some(IndexMap::new()));
            assert!(meta.checksum.is_none_or(|x| x.is_empty()));
        }
    }

    Ok(())
}

#[test]
fn exclude_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    for kind in ["project-json", "model-toml"] {
        let (_temp_dir, cwd) = init_of_kind(kind)?;
        let before = manifest_files(&cwd)?;

        let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

        let expected = if kind == "project-json" {
            "could not find file `test.sysml` in project metadata"
        } else {
            "`test.sysml` is not listed in `sources`"
        };
        out.assert()
            .failure()
            .stderr(predicates::str::contains(expected));

        assert_eq!(before, manifest_files(&cwd)?, "{kind}");
        if kind == "project-json" {
            let meta: InterchangeProjectMetadataRaw =
                serde_json::from_reader(fs::File::open(cwd.join(".meta.json"))?)?;

            assert_eq!(meta.index, Some(IndexMap::new()));
            assert!(meta.checksum.is_none_or(|x| x.is_empty()));
        }
    }

    Ok(())
}

// --- `model.toml` projects ---------------------------------------------

/// `sysand init --kind model-toml` of `acme/widgets`, in a fresh directory.
fn toml_project()
-> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--kind",
            "model-toml",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "--version",
            "1.2.3",
        ],
        None,
    )?;
    out.assert().success();
    Ok((temp_dir, cwd))
}

fn manifest(cwd: &camino::Utf8Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(fs::read_to_string(cwd.join("model.toml"))?)
}

#[test]
fn include_appends_to_sources() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .success();

    // `init` writes `sources = []`, and `sources` is written one entry per
    // line.
    assert!(manifest(&cwd)?.contains("sources = [\n    \"test.sysml\",\n]\n"));
    // "Neither extracts symbols or hashes files, as both maps are
    // computed at build."
    assert!(!exists(cwd.join(".meta.json")));
    Ok(())
}

#[test]
fn include_then_exclude_leaves_the_manifest_as_it_was() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    let before = manifest(&cwd)?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .success();
    run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?
        .assert()
        .success();

    assert_eq!(before, manifest(&cwd)?);
    Ok(())
}

#[test]
fn include_keeps_the_comments_in_the_manifest() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    let commented = manifest(&cwd)?.replace(
        "sources = []",
        "# the files this project is made of\nsources = []",
    );
    fs::write(cwd.join("model.toml"), &commented)?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .success();

    assert!(manifest(&cwd)?.contains("# the files this project is made of"));
    Ok(())
}

#[test]
fn include_refuses_a_second_time() -> Result<(), Box<dyn std::error::Error>> {
    // The one place the two formats differ in kind: a json project
    // re-includes to refresh its metadata, a toml one has none to
    // refresh and the second call is a duplicate.
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .success();
    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .failure();
    Ok(())
}

#[test]
fn include_refuses_compute_checksum() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "`checksum` is computed when the KPAR is built",
        ));
    Ok(())
}

#[test]
fn include_refuses_no_index_symbols() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml", "--no-index-symbols"], None)?
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "`index` is computed when the KPAR is built",
        ));
    Ok(())
}

#[test]
fn exclude_refuses_a_path_that_is_not_listed() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;

    run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "`test.sysml` is not listed in `sources`",
        ));
    Ok(())
}

/// Through the CLI: a file the project's metamodel
/// does not admit is refused before `sources` is touched.
#[test]
fn include_refuses_a_language_the_metamodel_does_not_admit()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--kind",
            "model-toml",
            "--metamodel",
            "https://www.omg.org/spec/KerML/20250201",
            "--publisher",
            "acme",
            "--name",
            "widgets",
        ],
        None,
    )?;
    out.assert().success();
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is a SysML file, which the metamodel `https://www.omg.org/spec/KerML/20250201` \
             does not admit",
        ));
    assert!(
        !manifest(&cwd)?.contains("test.sysml"),
        "nothing was written"
    );

    // A KerML file in the same project is included.
    fs::write(cwd.join("test.kerml"), b"package P;\n")?;
    run_sysand_in(&cwd, ["include", "test.kerml"], None)?
        .assert()
        .success();
    assert!(manifest(&cwd)?.contains("test.kerml"));
    Ok(())
}

/// `exclude` leaves a hand-commented manifest's comments and key order
/// intact.
#[test]
fn exclude_keeps_the_comments_and_key_order() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    let commented = manifest(&cwd)?.replace(
        "sources = []",
        "# the files this project is made of\nsources = []",
    );
    fs::write(cwd.join("model.toml"), &commented)?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .success();

    let before = manifest(&cwd)?;
    run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?
        .assert()
        .success();
    let after = manifest(&cwd)?;

    assert!(
        after.contains("# the files this project is made of"),
        "the comment survives exclude:\n{after}"
    );
    assert_eq!(
        keys(&before),
        keys(&after),
        "exclude must not reorder the keys"
    );
    Ok(())
}

/// The top-level key names, in the order the file writes them.
fn keys(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            (!line.starts_with('#') && line.contains(" = "))
                .then(|| line.split(" = ").next().unwrap_or_default().to_owned())
        })
        .collect()
}

/// A directory holding both a `.project.json` and a `model.toml` is not a
/// project, so neither command edits it.
#[test]
fn include_and_exclude_refuse_a_directory_holding_both_manifests()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    fs::write(cwd.join(".project.json"), "{}")?;
    let before = manifest(&cwd)?;

    for args in [["include", "test.sysml"], ["exclude", "test.sysml"]] {
        run_sysand_in(&cwd, args, None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "has both `model.toml` and `.project.json`",
            ));
    }
    assert_eq!(before, manifest(&cwd)?);
    Ok(())
}

/// A `model.toml` without `sources` is not a manifest Sysand reads, and the
/// error names the missing key.
#[test]
fn include_refuses_a_manifest_without_sources() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = toml_project()?;
    fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    let without_sources = manifest(&cwd)?.replace("sources = []\n", "");
    fs::write(cwd.join("model.toml"), &without_sources)?;

    run_sysand_in(&cwd, ["include", "test.sysml"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("sources"));
    assert_eq!(without_sources, manifest(&cwd)?);
    Ok(())
}
