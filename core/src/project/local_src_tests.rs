// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::LocalSrcProject;
use crate::project::ProjectRead as _;

fn write_project_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".project.json"), content).expect("write .project.json");
}

fn write_meta_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".meta.json"), content).expect("write .meta.json");
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
