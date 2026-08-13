// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::{LocalSrcError, LocalSrcProject};
use crate::project::ProjectRead as _;

fn write_project_json(dir: &camino::Utf8Path, content: &str) {
    std::fs::write(dir.join(".project.json"), content).expect("write .project.json");
}

#[test]
fn publisher_match_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(
        dir.path(),
        r#"{"name":"my-project","publisher":"acme","version":"1.0.0"}"#,
    );

    let project =
        LocalSrcProject::new_for_solve(dir.path(), None, Some("acme".into()), "my-project".into());

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

    let project = LocalSrcProject::new_for_solve(
        dir.path(),
        None,
        Some("expected-publisher".into()),
        "my-project".into(),
    );

    let err = project.get_project().unwrap_err();
    assert!(
        matches!(
            &err,
            LocalSrcError::PublisherMismatch {
                expected,
                actual,
            } if expected.as_deref() == Some("expected-publisher")
              && actual.as_deref() == Some("actual-publisher")
        ),
        "unexpected error: {err}"
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

    // new_for_solve with publisher=None sets expected_publisher=Some(None),
    // i.e. the project is expected to have no publisher.
    let project = LocalSrcProject::new_for_solve(dir.path(), None, None, "my-project".into());

    let err = project.get_project().unwrap_err();
    assert!(
        matches!(
            &err,
            LocalSrcError::PublisherMismatch {
                expected,
                actual,
            } if expected.is_none() && actual.as_deref() == Some("surprise")
        ),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn no_publisher_expected_and_absent_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"my-project","version":"1.0.0"}"#);

    let project = LocalSrcProject::new_for_solve(dir.path(), None, None, "my-project".into());

    let (info, _) = project.get_project()?;
    assert!(info.unwrap().publisher.is_none());
    Ok(())
}

#[test]
fn name_match_succeeds() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"correct-name","version":"1.0.0"}"#);

    let project = LocalSrcProject::new_for_solve(dir.path(), None, None, "correct-name".into());

    let (info, _) = project.get_project()?;
    assert_eq!(info.unwrap().name, "correct-name");
    Ok(())
}

#[test]
fn name_mismatch_returns_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    write_project_json(dir.path(), r#"{"name":"actual-name","version":"1.0.0"}"#);

    let project = LocalSrcProject::new_for_solve(dir.path(), None, None, "expected-name".into());

    let err = project.get_project().unwrap_err();
    assert!(
        matches!(
            &err,
            LocalSrcError::NameMismatch {
                expected,
                actual,
            } if expected == "expected-name" && actual == "actual-name"
        ),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn no_project_json_skips_checks() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    // No .project.json written — publisher/name checks should not run.

    let project =
        LocalSrcProject::new_for_solve(dir.path(), None, Some("anyone".into()), "anything".into());

    let (info, _) = project.get_project()?;
    assert!(info.is_none());
    Ok(())
}
