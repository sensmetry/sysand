// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Write;

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use sysand_core::model::{
    InterchangeProjectChecksumRaw, InterchangeProjectMetadataRaw, KerMlChecksumAlg,
};
// use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn include_and_exclude_simple() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_and_exclude",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        IndexMap::from([("P".to_string(), "test.sysml".to_string()),])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert!(meta.index.is_empty());
    assert!(meta.checksum.map(|x| x.is_empty()).unwrap_or(true));

    Ok(())
}

#[test]
fn include_no_checksum() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_no_checksum",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        IndexMap::from([("P".to_string(), "test.sysml".to_string()),])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "".to_string(),
                algorithm: KerMlChecksumAlg::None.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_no_index() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_and_exclude",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    out.assert().success();

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
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    assert!(meta.index.is_empty());

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_empty_and_update() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_and_exclude",
        ],
        None,
    )?;

    let mut sysml_file = std::fs::File::create(cwd.join("test.sysml"))?;
    sysml_file.sync_all()?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    assert!(meta.index.is_empty());

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    sysml_file.write_all(b"package P;\n")?;
    sysml_file.sync_all()?;

    let out = run_sysand_in(&cwd, ["include", "test.sysml", "--compute-checksum"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    assert_eq!(
        meta.index,
        IndexMap::from([("P".to_string(), "test.sysml".to_string()),])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_and_exclude_both_nested() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_and_exclude",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;
    std::fs::create_dir(cwd.join("extra"))?;
    std::fs::write(cwd.join("extra").join("test.sysml"), b"package Extra;\n")?;

    out.assert().success();

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
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert_eq!(
        meta.index,
        IndexMap::from([
            ("Extra".to_string(), "extra/test.sysml".to_string()),
            ("P".to_string(), "test.sysml".to_string()),
        ])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([
            (
                "extra/test.sysml".to_string(),
                InterchangeProjectChecksumRaw {
                    value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                        .to_string(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
            (
                "test.sysml".to_string(),
                InterchangeProjectChecksumRaw {
                    value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                        .to_string(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
        ])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml", "extra/test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert!(meta.index.is_empty());
    assert!(meta.checksum.map(|x| x.is_empty()).unwrap_or(true));

    Ok(())
}

#[test]
fn include_and_exclude_single_nested() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_and_exclude_single_nested",
        ],
        None,
    )?;

    std::fs::write(cwd.join("test.sysml"), b"package P;\n")?;

    std::fs::create_dir(cwd.join("extra"))?;

    std::fs::write(cwd.join("extra").join("test.sysml"), b"package Extra;\n")?;

    out.assert().success();

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
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert_eq!(
        meta.index,
        IndexMap::from([
            ("Extra".to_string(), "extra/test.sysml".to_string()),
            ("P".to_string(), "test.sysml".to_string()),
        ])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([
            (
                "extra/test.sysml".to_string(),
                InterchangeProjectChecksumRaw {
                    value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                        .to_string(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
            (
                "test.sysml".to_string(),
                InterchangeProjectChecksumRaw {
                    value: "b4ee9d8a3ffb51787bd30ab1a74c2333565fd2b8be1334e827c5937f44d54dd8"
                        .to_string(),
                    algorithm: KerMlChecksumAlg::Sha256.into(),
                }
            ),
        ])
    );

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

    out.assert().success();

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert_eq!(
        meta.index,
        IndexMap::from([("Extra".to_string(), "extra/test.sysml".to_string()),])
    );

    assert_eq!(
        meta.checksum.unwrap(),
        IndexMap::from([(
            "extra/test.sysml".to_string(),
            InterchangeProjectChecksumRaw {
                value: "d9c23ead98b668976f69c19b0500b89ba1acd0da4d78789f97195781ee02e6fc"
                    .to_string(),
                algorithm: KerMlChecksumAlg::Sha256.into(),
            }
        ),])
    );

    Ok(())
}

#[test]
fn include_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "include_nonexistent",
        ],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["include", "test.sysml"], None)?;

    let expected_error = if cfg!(windows) {
        "The system cannot find the file specified"
    } else {
        "No such file or directory"
    };
    out.assert()
        .failure()
        .stderr(predicates::str::contains(expected_error));

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert!(meta.index.is_empty());
    assert!(meta.checksum.map(|x| x.is_empty()).unwrap_or(true));

    Ok(())
}

#[test]
fn exclude_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--name",
            "exclude_nonexistent",
        ],
        None,
    )?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["exclude", "test.sysml"], None)?;

    out.assert().failure().stderr(predicates::str::contains(
        "could not find file `test.sysml` in project metadata",
    ));

    let meta: InterchangeProjectMetadataRaw =
        serde_json::from_reader(std::fs::File::open(cwd.join(".meta.json"))?)?;

    // let meta = meta.validate()?;

    assert!(meta.index.is_empty());
    assert!(meta.checksum.map(|x| x.is_empty()).unwrap_or(true));

    Ok(())
}
