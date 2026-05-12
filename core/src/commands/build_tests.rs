// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino_tempfile::tempdir;

use super::read_optional_project_file;
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
