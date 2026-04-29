// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

#[test]
fn deserialize_with_meta_metamodel_date() {
    let json = r#"{
            "projects": [
                {"path": "p1", "iris": ["urn:test:p1"]}
            ],
            "meta": {
                "metamodelDate": "20250201"
            }
        }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let info = WorkspaceInfo::try_from(raw).unwrap();
    assert!(info.meta.is_some());
    assert_eq!(
        info.meta.unwrap().metamodel_date.as_deref(),
        Some("20250201")
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
