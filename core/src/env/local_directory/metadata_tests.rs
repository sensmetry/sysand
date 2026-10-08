// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;
use std::assert_matches;

fn minimal_toml(path: &str, editable: bool) -> String {
    format!(
        r#"version = "0.2"

[[project]]
name = "Example"
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
    assert_matches!(
        err,
        ParseError::NonNormalizedProjectPath(_),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_cur_dir_component_is_rejected() {
    let toml = minimal_toml("./subdir", false);
    let err = EnvMetadata::from_str(&toml).unwrap_err();
    assert_matches!(
        err,
        ParseError::NonNormalizedProjectPath(_),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_absolute_path_is_rejected() {
    let toml = minimal_toml("/absolute/path", false);
    let err = EnvMetadata::from_str(&toml).unwrap_err();
    assert_matches!(
        err,
        ParseError::AbsoluteProjectPath(_),
        "unexpected error: {err}"
    );
}

#[test]
fn non_editable_normal_relative_path_is_accepted() {
    let toml = minimal_toml("subdir/project", false);
    EnvMetadata::from_str(&toml).unwrap();
}

#[test]
fn editable_project_with_parent_dir_is_accepted() {
    let toml = minimal_toml("../workspace-project", true);
    EnvMetadata::from_str(&toml).unwrap();
}

#[test]
fn unsupported_version_is_rejected() {
    let toml = r#"version = "99.0""#;
    let err = EnvMetadata::from_str(toml).unwrap_err();
    assert_matches!(
        err,
        ParseError::UnsupportedVersion(_),
        "unexpected error: {err}"
    );
}

#[test]
fn old_version_0_0_is_rejected() {
    let toml = r#"version = "0.0""#;
    let err = EnvMetadata::from_str(toml).unwrap_err();
    assert_matches!(
        &err,
        ParseError::UnsupportedVersion(v) if v == "0.0",
        "unexpected error: {err}"
    );
}

const METADATA_0_1: &str = r#"version = "0.1"

[[project]]
publisher = "ACME Inc."
name = "App"
version = "1.0.0"
path = "."
usages = [
    "urn:sysand:ACME%20Inc./Lib",
    "pkg:sysand/acme-labs/util",
]
editable = true
workspace = true

[[project]]
publisher = "ACME Inc."
name = "Lib"
version = "1.0.0"
path = "lib/ACME Inc..Lib_1.0.0"
identifiers = [
    "urn:sysand:ACME%20Inc./Lib",
    "urn:kpar:lib",
]
usages = ["urn:sysand:Ąžuolas/Šaknis"]

[[project]]
publisher = "Ąžuolas"
name = "Šaknis"
version = "1.0.0"
path = "lib/Ąžuolas.Šaknis_1.0.0"
identifiers = ["urn:sysand:Ąžuolas/Šaknis"]

[[project]]
publisher = "Acme Labs"
name = "Util"
version = "1.0.0"
path = "lib/acme-labs.util_1.0.0"
identifiers = ["pkg:sysand/acme-labs/util"]
"#;

#[test]
fn version_0_1_is_migrated_to_current_version() {
    let meta = EnvMetadata::from_str(METADATA_0_1).unwrap();
    assert_eq!(meta.version, CURRENT_METADATA_VERSION);

    let identifiers: Vec<_> = meta
        .projects
        .iter()
        .map(|p| p.identifiers.as_slice())
        .collect();
    assert_eq!(
        identifiers,
        [
            &[][..],
            &[
                "urn:sysand:acme-inc/lib".to_owned(),
                "urn:kpar:lib".to_owned()
            ],
            &["urn:sysand:ąžuolas/šaknis".to_owned()],
            &["pkg:sysand/acme-labs/util".to_owned()],
        ]
    );
    let usages: Vec<_> = meta.projects.iter().map(|p| p.usages.as_slice()).collect();
    assert_eq!(
        usages,
        [
            &[
                "urn:sysand:acme-inc/lib".to_owned(),
                "pkg:sysand/acme-labs/util".to_owned()
            ][..],
            &["urn:sysand:ąžuolas/šaknis".to_owned()],
            &[],
            &[],
        ]
    );
    // Installed projects stay where they are
    assert_eq!(meta.projects[1].path, "lib/ACME Inc..Lib_1.0.0");

    // Found by the new identifier, and written as the current version
    assert!(
        meta.find_project_version("urn:sysand:acme-inc/lib", "1.0.0")
            .is_some()
    );
    let text = meta.to_string();
    assert!(
        text.contains(&format!("version = \"{CURRENT_METADATA_VERSION}\"")),
        "{text}"
    );
    assert!(!text.contains("ACME%20Inc."), "{text}");
}

// --- Env identifiers ---

#[test]
fn env_project_with_urn_kpar_identifier_is_found() {
    let toml = r#"version = "0.2"

[[project]]
name = "my-dep"
version = "1.0.0"
path = "projects/my-dep"
identifiers = [
    "urn:kpar:my-dep",
]
"#;
    let meta = EnvMetadata::from_str(toml).unwrap();
    let found = meta.find_project_version("urn:kpar:my-dep", "1.0.0");
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "my-dep");
}

#[test]
fn env_project_with_urn_sysand_identifier_is_found() {
    // urn:sysand: is the non-PURL, non-URL form produced by typed usages
    // with publishers/names that cannot be represented as a PURL (e.g. too short)
    let toml = r#"version = "0.2"

[[project]]
publisher = "ab"
name = "my-lib"
version = "1.0.0"
path = "projects/my-lib"
identifiers = [
    "urn:sysand:ab/my-lib",
]
"#;
    let meta = EnvMetadata::from_str(toml).unwrap();
    let found = meta.find_project_version("urn:sysand:ab/my-lib", "1.0.0");
    assert!(found.is_some());
    let project = found.unwrap();
    assert_eq!(project.name, "my-lib");
    assert_eq!(project.publisher.as_deref(), Some("ab"));
}

#[test]
fn env_project_with_urn_sysand_identifier_has_correct_usages() {
    let toml = r#"version = "0.2"

[[project]]
name = "consumer"
version = "2.0.0"
path = "projects/consumer"
usages = [
    "urn:sysand:ab/my-lib",
]

[[project]]
publisher = "ab"
name = "my-lib"
version = "1.0.0"
path = "projects/my-lib"
identifiers = [
    "urn:sysand:ab/my-lib",
]
"#;
    let meta = EnvMetadata::from_str(toml).unwrap();

    // consumer has no identifiers, look it up by name
    let consumer = meta.projects.iter().find(|p| p.name == "consumer").unwrap();
    assert_eq!(consumer.usages, vec!["urn:sysand:ab/my-lib"]);

    let dep = meta
        .find_project_version("urn:sysand:ab/my-lib", "1.0.0")
        .unwrap();
    assert_eq!(dep.name, "my-lib");
}
