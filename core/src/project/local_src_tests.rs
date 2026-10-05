// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use std::assert_matches;

use super::{LocalSrcError, LocalSrcProject};
use crate::project::ProjectRead as _;

fn write_project_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".project.json"), content).expect("write .project.json");
}

/// The project at `dir`, expected to be `publisher`/`name` as sync expects
/// what the lockfile records. The checksum is not checked when reading
fn expecting(dir: &camino::Utf8Path, publisher: Option<&str>, name: &str) -> LocalSrcProject {
    LocalSrcProject::new_for_sync(
        dir,
        None,
        publisher.map(Into::into),
        name.into(),
        String::new(),
    )
}

#[test]
fn publisher_match_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"my-project","publisher":"acme","version":"1.0.0"}"#,
    );

    let project = expecting(dir.path(), Some("acme"), "my-project");

    let (info, _) = project.get_project()?;
    assert_eq!(info.unwrap().publisher.as_deref(), Some("acme"));
    Ok(())
}

#[test]
fn publisher_mismatch_returns_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"my-project","publisher":"actual-publisher","version":"1.0.0"}"#,
    );

    let project = expecting(dir.path(), Some("expected-publisher"), "my-project");

    let err = project.get_project().unwrap_err();
    assert_matches!(
        &err,
        LocalSrcError::PublisherMismatch {
            expected,
            actual,
        } if expected.as_deref() == Some("expected-publisher")
          && actual.as_deref() == Some("actual-publisher")
    );
    Ok(())
}

#[test]
fn expects_no_publisher_but_project_has_one() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"my-project","publisher":"surprise","version":"1.0.0"}"#,
    );

    // No publisher expected means the project is expected to have none
    let project = expecting(dir.path(), None, "my-project");

    let err = project.get_project().unwrap_err();
    assert_matches!(
        &err,
        LocalSrcError::PublisherMismatch {
            expected,
            actual,
        } if expected.is_none() && actual.as_deref() == Some("surprise")
    );
    Ok(())
}

#[test]
fn no_publisher_expected_and_absent_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"my-project","version":"1.0.0"}"#);

    let project = expecting(dir.path(), None, "my-project");

    let (info, _) = project.get_project()?;
    assert!(info.unwrap().publisher.is_none());
    Ok(())
}

#[test]
fn name_match_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"correct-name","version":"1.0.0"}"#);

    let project = expecting(dir.path(), None, "correct-name");

    let (info, _) = project.get_project()?;
    assert_eq!(info.unwrap().name, "correct-name");
    Ok(())
}

#[test]
fn name_mismatch_returns_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"actual-name","version":"1.0.0"}"#);

    let project = expecting(dir.path(), None, "expected-name");

    let err = project.get_project().unwrap_err();
    assert_matches!(
            &err,
            LocalSrcError::NameMismatch {
                expected,
                actual,
            } if expected == "expected-name" && actual == "actual-name"
    );
    Ok(())
}

#[test]
fn no_project_json_skips_checks() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    // No .project.json written — publisher/name checks should not run.

    let project = expecting(dir.path(), Some("anyone"), "anything");

    let (info, _) = project.get_project()?;
    assert!(info.is_none());
    Ok(())
}
