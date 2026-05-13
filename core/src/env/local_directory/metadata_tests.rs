// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

fn minimal_toml(path: &str, editable: bool) -> String {
    format!(
        r#"version = "0.1"

[[project]]
version = "1.0.0"
path = "{path}"
editable = {editable}
"#
    )
}

#[test]
fn non_editable_parent_dir_component_is_rejected() {
    let toml = minimal_toml("../escape", false);
    let err = EnvMetadata::from_str(&toml).unwrap_err();
    assert!(
        matches!(err, ParseError::NonNormalizedProjectPath(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_cur_dir_component_is_rejected() {
    let toml = minimal_toml("./subdir", false);
    let err = EnvMetadata::from_str(&toml).unwrap_err();
    assert!(
        matches!(err, ParseError::NonNormalizedProjectPath(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_absolute_path_is_rejected() {
    let toml = minimal_toml("/absolute/path", false);
    let err = EnvMetadata::from_str(&toml).unwrap_err();
    assert!(
        matches!(err, ParseError::AbsoluteProjectPath(_)),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_normal_relative_path_is_accepted() {
    let toml = minimal_toml("subdir/project", false);
    assert!(EnvMetadata::from_str(&toml).is_ok());
}

#[test]
fn editable_project_with_parent_dir_is_accepted() {
    let toml = minimal_toml("../workspace-project", true);
    assert!(EnvMetadata::from_str(&toml).is_ok());
}

#[test]
fn unsupported_version_is_rejected() {
    let toml = r#"version = "99.0""#;
    let err = EnvMetadata::from_str(toml).unwrap_err();
    assert!(
        matches!(err, ParseError::UnsupportedVersion(_)),
        "unexpected error: {err}"
    );
}
