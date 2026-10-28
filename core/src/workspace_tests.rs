// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;
use std::assert_matches;

#[test]
fn deserialize_with_meta_metamodel() {
    let json = r#"{
            "projects": [
                {"path": "p1", "iris": ["urn:test:p1"]}
            ],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/20250201"
            }
        }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let info = WorkspaceInfo::try_from(raw).unwrap();
    assert!(info.meta.is_some());
    assert_eq!(
        info.meta.unwrap().metamodel.unwrap().to_string(),
        "https://www.omg.org/spec/SysML/20250201"
    );
}

#[test]
fn deserialize_without_meta() {
    let json = r#"{
            "projects": [
                {"path": "p1", "iris": ["urn:test:p1"]}
            ]
        }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let info = WorkspaceInfo::try_from(raw).unwrap();
    assert!(info.meta.is_none());
}

#[test]
fn deserialize_invalid_metamodel_iri() {
    let json = r#"{
            "projects": [],
            "meta": {
                "metamodel": "not a valid iri {"
            }
        }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let result = WorkspaceInfo::try_from(raw);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_matches!(
        err,
        WorkspaceValidationError::MalformedMetamodel(MetamodelParseError::NotUrl {
            value: _,
            error: _
        })
    );
}

#[test]
fn deserialize_malformed_metamodel_release() {
    let json = r#"{
            "projects": [],
            "meta": {
                "metamodel": "https://www.omg.org/spec/SysML/2025"
            }
        }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    assert_matches!(
        WorkspaceInfo::try_from(raw),
        Err(WorkspaceValidationError::MalformedMetamodel(_))
    );
}
