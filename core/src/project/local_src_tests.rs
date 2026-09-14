// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use std::assert_matches;

use super::{LocalSrcError, LocalSrcProject, ProjectFormat, detect_format};
use crate::project::ProjectRead as _;

fn write_project_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".project.json"), content).expect("write .project.json");
}

fn write_meta_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".meta.json"), content).expect("write .meta.json");
}

fn write_model_toml(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join("model.toml"), content).expect("write model.toml");
}

fn read_error(dir: &camino::Utf8Path) -> String {
    LocalSrcProject::new_access(dir, None)
        .get_project()
        .expect_err("the project should not be readable")
        .to_string()
}

/// A key a manifest must carry is named by Sysand, not by serde.
#[test]
fn meta_json_without_index_names_the_key() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"p","version":"1.0.0"}"#);
    write_meta_json(dir.path(), r#"{"created":"2026-01-01T00:00:00Z"}"#);

    let error = read_error(dir.path());
    assert!(
        error.contains("missing key `index` in `.meta.json`"),
        "unexpected error: {error}"
    );
    assert!(
        !error.contains("missing field"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn project_json_without_name_names_every_missing_key() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"publisher":"acme"}"#);

    let error = read_error(dir.path());
    assert!(
        error.contains("missing key `name` in `.project.json`"),
        "unexpected error: {error}"
    );
    assert!(
        error.contains("missing key `version` in `.project.json`"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// The metadata is read first, so its metamodel can be named as a reason
/// for a key the information document does not carry.
#[test]
fn an_unrecognized_metamodel_is_named_as_a_possible_reason()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"publisher":"acme"}"#);
    write_meta_json(
        dir.path(),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z","metamodel":"https://example.com/x"}"#,
    );

    let error = read_error(dir.path());
    assert!(
        error.contains("this may be because the project uses a metamodel"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// A document that is not json at all is still serde's to report: it
/// knows where the text stops making sense.
#[test]
fn a_manifest_that_is_not_json_still_fails_through_serde() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempdir()?;
    write_project_json(dir.path(), "not json at all");

    let error = read_error(dir.path());
    assert!(
        error.contains("failed to deserialize"),
        "unexpected error: {error}"
    );
    Ok(())
}

/// An unknown key does not stop a read; it is reported and dropped.
#[test]
fn an_unknown_key_does_not_stop_a_read() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"p","version":"1.0.0","x-unknown":true}"#,
    );

    let (info, _) = LocalSrcProject::new_access(dir.path(), None).get_project()?;
    assert_eq!(info.expect("a project").name, "p");
    Ok(())
}

// ----- which manifest a directory holds -----

#[test]
fn an_empty_directory_has_neither_format() {
    let dir = tempdir().expect("a temporary directory");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Neither
    );
}

#[test]
fn a_project_json_alone_is_the_json_format() {
    let dir = tempdir().expect("a temporary directory");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Json
    );
}

/// The json side is *either* file, not both: a directory holding only a
/// `.meta.json` is a broken json project, not a directory without one.
#[test]
fn a_meta_json_alone_is_the_json_format() {
    let dir = tempdir().expect("a temporary directory");
    write_meta_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Json
    );
}

#[test]
fn a_model_toml_alone_is_the_toml_format() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Toml
    );
}

#[test]
fn a_directory_with_both_is_ambiguous() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_project_json(dir.path(), "{}");
    assert_eq!(
        detect_format(dir.path()).expect("readable"),
        ProjectFormat::Both
    );
}

/// model-toml.md "Mixed manifests": "current project has toml and json
/// configs: error out that it's unclear if the project is meant to be a
/// non-zipped KPAR, or dev project".
#[test]
fn an_ambiguous_directory_is_refused_when_it_is_read() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_project_json(dir.path(), "{}");
    write_meta_json(dir.path(), "{}");
    assert_eq!(
        read_error(dir.path()),
        format!(
            "`{}` has both `model.toml` and `.project.json`;\n         \
             it is unclear whether it is a non-zipped KPAR or a development project",
            dir.path()
        )
    );
}

/// The message names the json file that is actually there.
#[test]
fn an_ambiguous_directory_names_the_json_file_it_has() {
    let dir = tempdir().expect("a temporary directory");
    write_model_toml(dir.path(), "manifest-version = 1\n");
    write_meta_json(dir.path(), "{}");
    assert!(
        read_error(dir.path()).starts_with(&format!(
            "`{}` has both `model.toml` and `.meta.json`;",
            dir.path()
        )),
        "unexpected message: {}",
        read_error(dir.path())
    );
}
