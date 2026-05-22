// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;
use crate::model::{
    InterchangeProjectInfoWithInheritRaw, InterchangeProjectMetadataWithInheritRaw,
    WorkspaceInherit,
};

// ---------------------------------------------------------------------------
// Existing deserialization tests
// ---------------------------------------------------------------------------

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
        info.meta.unwrap().metamodel.unwrap().as_str(),
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
    assert!(matches!(err, WorkspaceValidationError::InvalidIri(..)));
}

// ---------------------------------------------------------------------------
// Workspace project defaults and groups deserialization
// ---------------------------------------------------------------------------

#[test]
fn deserialize_with_project_defaults() {
    let json = r#"{
        "projects": [],
        "project": {
            "version": "1.2.3",
            "publisher": "Acme",
            "license": "MIT"
        }
    }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let info = WorkspaceInfo::try_from(raw).unwrap();
    let proj = info.project.as_ref().unwrap();
    assert_eq!(proj.version.as_deref(), Some("1.2.3"));
    assert_eq!(proj.publisher.as_deref(), Some("Acme"));
    assert_eq!(proj.license.as_deref(), Some("MIT"));
}

#[test]
fn deserialize_with_presets() {
    let json = r#"{
        "projects": [],
        "presets": {
            "kerml": {
                "project": { "version": "1.0.0" },
                "meta": { "metamodel": "https://www.omg.org/spec/KerML/20250201" }
            },
            "sysml": {
                "project": { "version": "2.0.0" },
                "meta": { "metamodel": "https://www.omg.org/spec/SysML/20250201" }
            }
        }
    }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let info = WorkspaceInfo::try_from(raw).unwrap();
    let presets = info.presets.as_ref().unwrap();
    assert_eq!(presets.len(), 2);
    let kerml = presets.get("kerml").unwrap();
    assert_eq!(
        kerml.project.as_ref().unwrap().version.as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        kerml.meta.as_ref().unwrap().metamodel.as_deref(),
        Some("https://www.omg.org/spec/KerML/20250201")
    );
}

// ---------------------------------------------------------------------------
// WorkspaceInherit serde round-trips
// ---------------------------------------------------------------------------

#[test]
fn workspace_inherit_literal_deserializes() {
    let json = r#""1.0.0""#;
    let val: WorkspaceInherit<String> = serde_json::from_str(json).unwrap();
    assert_eq!(val, WorkspaceInherit::Literal("1.0.0".to_string()));
}

#[test]
fn workspace_inherit_default_preset_deserializes() {
    let json = r#"{"preset": "default"}"#;
    let val: WorkspaceInherit<String> = serde_json::from_str(json).unwrap();
    assert!(matches!(val, WorkspaceInherit::Preset { ref preset } if preset == "default"));
}

#[test]
fn workspace_inherit_named_preset_deserializes() {
    let json = r#"{"preset": "kerml"}"#;
    let val: WorkspaceInherit<String> = serde_json::from_str(json).unwrap();
    assert!(matches!(val, WorkspaceInherit::Preset { ref preset } if preset == "kerml"));
}

// ---------------------------------------------------------------------------
// resolve_project_info
// ---------------------------------------------------------------------------

fn make_workspace_info(
    root_version: Option<&str>,
    presets: &[(&str, &str, Option<&str>)], // (name, version, metamodel)
    root_metamodel: Option<&str>,
) -> WorkspaceInfo {
    let project = root_version.map(|v| WorkspaceProjectDefaultsRaw {
        version: Some(v.to_string()),
        publisher: None,
        license: None,
    });

    let presets_map: Option<indexmap::IndexMap<String, WorkspacePresetEntryRaw>> =
        if presets.is_empty() {
            None
        } else {
            Some(
                presets
                    .iter()
                    .map(|(name, version, metamodel)| {
                        (
                            name.to_string(),
                            WorkspacePresetEntryRaw {
                                project: Some(WorkspaceProjectDefaultsRaw {
                                    version: Some(version.to_string()),
                                    publisher: None,
                                    license: None,
                                }),
                                meta: metamodel.map(|m| WorkspaceMetaRaw {
                                    metamodel: Some(m.to_string()),
                                }),
                            },
                        )
                    })
                    .collect(),
            )
        };

    let meta = root_metamodel.map(|m| {
        use fluent_uri::Iri;
        WorkspaceMeta {
            metamodel: Some(Iri::parse(m.to_string()).unwrap()),
        }
    });

    WorkspaceInfo {
        projects: vec![],
        meta,
        project,
        presets: presets_map,
    }
}

fn make_project_info_raw(
    version: WorkspaceInherit<String>,
) -> InterchangeProjectInfoWithInheritRaw {
    InterchangeProjectInfoWithInheritRaw {
        name: "my-project".to_string(),
        publisher: None,
        description: None,
        version,
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    }
}

#[test]
fn resolve_project_info_literal_version() {
    let ws = make_workspace_info(None, &[], None);
    let raw = make_project_info_raw(WorkspaceInherit::Literal("0.5.0".to_string()));
    let resolved = resolve_project_info(raw, &ws).unwrap();
    assert_eq!(resolved.version, "0.5.0");
}

#[test]
fn resolve_project_info_default_preset() {
    let ws = make_workspace_info(Some("3.0.0"), &[], None);
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "default".to_string(),
    });
    let resolved = resolve_project_info(raw, &ws).unwrap();
    assert_eq!(resolved.version, "3.0.0");
}

#[test]
fn resolve_project_info_named_preset() {
    let ws = make_workspace_info(
        None,
        &[(
            "kerml",
            "1.0.0",
            Some("https://www.omg.org/spec/KerML/20250201"),
        )],
        None,
    );
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "kerml".to_string(),
    });
    let resolved = resolve_project_info(raw, &ws).unwrap();
    assert_eq!(resolved.version, "1.0.0");
}

#[test]
fn resolve_project_info_mixed_presets() {
    // version from "sysml", publisher from "kerml" — both resolve independently
    let ws_info = WorkspaceInfo {
        projects: vec![],
        meta: None,
        project: None,
        presets: Some({
            let mut m = indexmap::IndexMap::new();
            m.insert(
                "kerml".to_string(),
                WorkspacePresetEntryRaw {
                    project: Some(WorkspaceProjectDefaultsRaw {
                        version: Some("1.0.0".to_string()),
                        publisher: Some("KerML Corp".to_string()),
                        license: None,
                    }),
                    meta: None,
                },
            );
            m.insert(
                "sysml".to_string(),
                WorkspacePresetEntryRaw {
                    project: Some(WorkspaceProjectDefaultsRaw {
                        version: Some("2.0.0".to_string()),
                        publisher: None,
                        license: None,
                    }),
                    meta: None,
                },
            );
            m
        }),
    };
    let raw = InterchangeProjectInfoWithInheritRaw {
        name: "my-project".to_string(),
        publisher: Some(WorkspaceInherit::Preset {
            preset: "kerml".to_string(),
        }),
        description: None,
        version: WorkspaceInherit::Preset {
            preset: "sysml".to_string(),
        },
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };
    let resolved = resolve_project_info(raw, &ws_info).unwrap();
    assert_eq!(resolved.version, "2.0.0");
    assert_eq!(resolved.publisher.as_deref(), Some("KerML Corp"));
}

#[test]
fn resolve_project_info_unknown_preset_error() {
    let ws = make_workspace_info(None, &[], None);
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "nonexistent".to_string(),
    });
    let err = resolve_project_info(raw, &ws).unwrap_err();
    assert!(matches!(
        err,
        WorkspaceInheritanceError::UnknownPreset { .. }
    ));
}

#[test]
fn resolve_project_info_missing_root_default_error() {
    let ws = make_workspace_info(None, &[], None); // no project defaults
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "default".to_string(),
    });
    let err = resolve_project_info(raw, &ws).unwrap_err();
    assert!(matches!(
        err,
        WorkspaceInheritanceError::MissingRootDefault { .. }
    ));
}

#[test]
fn resolve_project_info_missing_preset_default_error() {
    // Preset exists but has no version
    let ws_info = WorkspaceInfo {
        projects: vec![],
        meta: None,
        project: None,
        presets: Some({
            let mut m = indexmap::IndexMap::new();
            m.insert(
                "kerml".to_string(),
                WorkspacePresetEntryRaw {
                    project: None, // no project defaults
                    meta: None,
                },
            );
            m
        }),
    };
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "kerml".to_string(),
    });
    let err = resolve_project_info(raw, &ws_info).unwrap_err();
    assert!(matches!(
        err,
        WorkspaceInheritanceError::MissingPresetDefault { .. }
    ));
}

#[test]
fn project_info_without_workspace_literal_passes() {
    let raw = make_project_info_raw(WorkspaceInherit::Literal("1.0.0".to_string()));
    let resolved = crate::workspace::project_info_without_workspace(raw).unwrap();
    assert_eq!(resolved.version, "1.0.0");
}

#[test]
fn project_info_without_workspace_ref_errors() {
    let raw = make_project_info_raw(WorkspaceInherit::Preset {
        preset: "default".to_string(),
    });
    let err = crate::workspace::project_info_without_workspace(raw).unwrap_err();
    assert!(matches!(err, WorkspaceInheritanceError::NoWorkspace { .. }));
}

// ---------------------------------------------------------------------------
// resolve_project_metadata
// ---------------------------------------------------------------------------

fn make_meta_raw(
    metamodel: Option<WorkspaceInherit<String>>,
) -> InterchangeProjectMetadataWithInheritRaw {
    InterchangeProjectMetadataWithInheritRaw {
        index: indexmap::IndexMap::new(),
        created: "2026-01-01T00:00:00Z".to_string(),
        metamodel,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    }
}

#[test]
fn resolve_project_metadata_no_metamodel() {
    let ws = make_workspace_info(None, &[], None);
    let raw = make_meta_raw(None);
    let resolved = crate::workspace::resolve_project_metadata(raw, &ws, "my-project").unwrap();
    assert!(resolved.metamodel.is_none());
}

#[test]
fn resolve_project_metadata_literal_metamodel() {
    let ws = make_workspace_info(None, &[], None);
    let raw = make_meta_raw(Some(WorkspaceInherit::Literal(
        "https://www.omg.org/spec/KerML/20250201".to_string(),
    )));
    let resolved = crate::workspace::resolve_project_metadata(raw, &ws, "my-project").unwrap();
    assert_eq!(
        resolved.metamodel.as_deref(),
        Some("https://www.omg.org/spec/KerML/20250201")
    );
}

#[test]
fn resolve_project_metadata_default_preset() {
    let ws = make_workspace_info(None, &[], Some("https://www.omg.org/spec/SysML/20250201"));
    let raw = make_meta_raw(Some(WorkspaceInherit::Preset {
        preset: "default".to_string(),
    }));
    let resolved = crate::workspace::resolve_project_metadata(raw, &ws, "my-project").unwrap();
    assert_eq!(
        resolved.metamodel.as_deref(),
        Some("https://www.omg.org/spec/SysML/20250201")
    );
}

#[test]
fn resolve_project_metadata_named_preset() {
    let ws = make_workspace_info(
        None,
        &[(
            "kerml",
            "1.0.0",
            Some("https://www.omg.org/spec/KerML/20250201"),
        )],
        None,
    );
    let raw = make_meta_raw(Some(WorkspaceInherit::Preset {
        preset: "kerml".to_string(),
    }));
    let resolved = crate::workspace::resolve_project_metadata(raw, &ws, "my-project").unwrap();
    assert_eq!(
        resolved.metamodel.as_deref(),
        Some("https://www.omg.org/spec/KerML/20250201")
    );
}

#[test]
fn project_metadata_without_workspace_literal_passes() {
    let raw = make_meta_raw(Some(WorkspaceInherit::Literal("some_iri".to_string())));
    let resolved = crate::workspace::project_metadata_without_workspace(raw, "my-project").unwrap();
    assert_eq!(resolved.metamodel.as_deref(), Some("some_iri"));
}

#[test]
fn project_metadata_without_workspace_ref_errors() {
    let raw = make_meta_raw(Some(WorkspaceInherit::Preset {
        preset: "default".to_string(),
    }));
    let err = crate::workspace::project_metadata_without_workspace(raw, "my-project").unwrap_err();
    assert!(matches!(err, WorkspaceInheritanceError::NoWorkspace { .. }));
}

// ---------------------------------------------------------------------------
// Workspace validation: reserved preset name and root+preset conflict
// ---------------------------------------------------------------------------

#[test]
fn reserved_preset_name_default_is_rejected() {
    let json = r#"{
        "projects": [],
        "presets": {
            "default": { "project": { "version": "1.0.0" } }
        }
    }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let err = WorkspaceInfo::try_from(raw).unwrap_err();
    assert!(matches!(err, WorkspaceValidationError::ReservedPresetName));
}

#[test]
fn root_and_preset_version_conflict_is_rejected() {
    let json = r#"{
        "projects": [],
        "project": { "version": "1.0.0" },
        "presets": {
            "kerml": { "project": { "version": "2.0.0" } }
        }
    }"#;
    let raw: WorkspaceInfoRaw = serde_json::from_str(json).unwrap();
    let err = WorkspaceInfo::try_from(raw).unwrap_err();
    assert!(matches!(
        err,
        WorkspaceValidationError::RootAndPresetConflict {
            field: "version",
            ..
        }
    ));
}
