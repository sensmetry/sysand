// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::{license_file_stems, read_optional_project_file};
use crate::project::utils::FsIoError;

#[test]
fn returns_none_when_project_root_is_none() {
    let result = read_optional_project_file(None, "README.md", "readme").unwrap();
    assert!(result.is_none());
}

#[test]
fn returns_none_when_file_does_not_exist() {
    let tmp = tempdir().unwrap();
    let result = read_optional_project_file(Some(tmp.path()), "README.md", "readme").unwrap();
    assert!(result.is_none());
}

#[test]
fn returns_content_when_file_exists() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("CHANGELOG.md"), b"# Changelog\n- entry\n").unwrap();

    let result = read_optional_project_file(Some(tmp.path()), "CHANGELOG.md", "changelog").unwrap();
    assert_eq!(result.as_deref(), Some("# Changelog\n- entry\n"));
}

#[test]
fn surfaces_non_not_found_io_errors() {
    let tmp = tempdir().unwrap();
    // A directory at the target path makes `read_to_string` fail with a
    // non-`NotFound` error, which the helper must propagate.
    std::fs::create_dir(tmp.path().join("README.md")).unwrap();

    let err = read_optional_project_file(Some(tmp.path()), "README.md", "readme").unwrap_err();
    match err {
        FsIoError::ReadFile(path, _) => assert_eq!(path, tmp.path().join("README.md")),
        other => panic!("expected FsIoError::ReadFile, got {other:?}"),
    }
}

fn stems(expr: &str) -> Vec<String> {
    license_file_stems(&spdx::Expression::parse(expr).unwrap())
}

#[test]
fn license_stems_single() {
    assert_eq!(stems("MIT"), vec!["MIT".to_string()]);
}

#[test]
fn license_stems_compound_or() {
    assert_eq!(
        stems("MIT OR Apache-2.0"),
        vec!["MIT".to_string(), "Apache-2.0".to_string()]
    );
}

#[test]
fn license_stems_compound_and() {
    assert_eq!(
        stems("MIT AND BSD-2-Clause"),
        vec!["MIT".to_string(), "BSD-2-Clause".to_string()]
    );
}

#[test]
fn license_stems_with_exception() {
    assert_eq!(
        stems("GPL-2.0-only WITH Classpath-exception-2.0"),
        vec![
            "GPL-2.0-only".to_string(),
            "Classpath-exception-2.0".to_string(),
        ]
    );
}

#[test]
fn license_stems_license_ref() {
    assert_eq!(
        stems("LicenseRef-MyCustom"),
        vec!["LicenseRef-MyCustom".to_string()]
    );
}

#[test]
fn license_stems_or_later_strips_plus() {
    // `MIT+` shares its license file with `MIT` per REUSE conventions —
    // the `+` does not appear in the bundled filename.
    assert_eq!(stems("MIT+"), vec!["MIT".to_string()]);
}

#[test]
fn license_stems_deduplicates() {
    assert_eq!(stems("MIT AND MIT"), vec!["MIT".to_string()]);
}
