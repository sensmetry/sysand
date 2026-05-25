// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, WorkspaceInherit};

use super::types::{WorkspaceInfo, WorkspacePresetEntryRaw};

#[derive(Error, Debug)]
pub enum WorkspaceInheritanceError {
    #[error(
        "project `{project}`: field `{field}` references preset `{preset}`, \
         but no such preset exists in `.workspace.json`"
    )]
    UnknownPreset {
        project: String,
        field: &'static str,
        preset: String,
    },

    #[error(
        "project `{project}`: field `{field}` uses {{\"preset\": \"default\"}}, \
         but `.workspace.json` has no `project.{field}` default"
    )]
    MissingRootDefault {
        project: String,
        field: &'static str,
    },

    #[error(
        "project `{project}`: field `{field}` uses {{\"preset\": \"{preset}\"}}, \
         but preset `{preset}` has no `project.{field}` default"
    )]
    MissingPresetDefault {
        project: String,
        field: &'static str,
        preset: String,
    },

    #[error("project `{project}` uses workspace inheritance but no `.workspace.json` was found")]
    NoWorkspace { project: String },
}

/// Resolve a single required `WorkspaceInherit<String>` field.
///
/// * `Literal(v)` — returned as-is.
/// * `{ "preset": "default" }` — calls `get_root_default(workspace)`; errors with
///   [`WorkspaceInheritanceError::MissingRootDefault`] if the workspace has no
///   value for this field.
/// * `{ "preset": "name" }` — looks up the named preset, then calls
///   `get_preset_default`; errors with [`WorkspaceInheritanceError::UnknownPreset`]
///   or [`WorkspaceInheritanceError::MissingPresetDefault`] as appropriate.
///
/// Returns the resolved string value and, when a named preset was used, the
/// preset name (used by callers that need to know which preset was resolved).
fn resolve_field<'a>(
    field: WorkspaceInherit<String>,
    field_name: &'static str,
    project_name: &str,
    workspace: &'a WorkspaceInfo,
    get_root_default: impl Fn(&'a WorkspaceInfo) -> Option<&'a str>,
    get_preset_default: impl Fn(&'a WorkspacePresetEntryRaw) -> Option<&'a str>,
) -> Result<(String, Option<String>), WorkspaceInheritanceError> {
    match field {
        WorkspaceInherit::Literal(v) => Ok((v, None)),
        WorkspaceInherit::Preset { preset } => {
            if preset == "default" {
                let value = get_root_default(workspace).ok_or_else(|| {
                    WorkspaceInheritanceError::MissingRootDefault {
                        project: project_name.to_string(),
                        field: field_name,
                    }
                })?;
                Ok((value.to_string(), None))
            } else {
                let entry = workspace
                    .presets
                    .as_ref()
                    .and_then(|p| p.get(&preset))
                    .ok_or_else(|| WorkspaceInheritanceError::UnknownPreset {
                        project: project_name.to_string(),
                        field: field_name,
                        preset: preset.clone(),
                    })?;
                let value = get_preset_default(entry).ok_or_else(|| {
                    WorkspaceInheritanceError::MissingPresetDefault {
                        project: project_name.to_string(),
                        field: field_name,
                        preset: preset.clone(),
                    }
                })?;
                Ok((value.to_string(), Some(preset)))
            }
        }
    }
}

/// Like [`resolve_field`], but for optional fields: `None` is passed through as
/// `(None, None)` without consulting the workspace.
fn resolve_optional_field<'a>(
    field: Option<WorkspaceInherit<String>>,
    field_name: &'static str,
    project_name: &str,
    workspace: &'a WorkspaceInfo,
    get_root_default: impl Fn(&'a WorkspaceInfo) -> Option<&'a str>,
    get_preset_default: impl Fn(&'a WorkspacePresetEntryRaw) -> Option<&'a str>,
) -> Result<(Option<String>, Option<String>), WorkspaceInheritanceError> {
    match field {
        None => Ok((None, None)),
        Some(f) => {
            let (v, g) = resolve_field(
                f,
                field_name,
                project_name,
                workspace,
                get_root_default,
                get_preset_default,
            )?;
            Ok((Some(v), g))
        }
    }
}

/// Resolve all workspace-inherit references in `.project.json`.
///
/// Each of `version`, `publisher`, and `license` may carry a
/// [`WorkspaceInherit`] value instead of a literal string. Root defaults are
/// read from [`WorkspaceInfo::project`]; group defaults from
/// [`WorkspaceGroupEntryRaw::project`].
///
/// Fields that are absent (`None`) are left as `None`; fields that carry a
/// literal value are passed through unchanged.
///
/// # Errors
///
/// Returns [`WorkspaceInheritanceError`] if a referenced group does not exist,
/// the requested default is absent, or `{ "workspace": false }` is used.
pub fn resolve_project_info(
    raw: crate::model::InterchangeProjectInfoWithInheritRaw,
    workspace: &WorkspaceInfo,
) -> Result<InterchangeProjectInfoRaw, WorkspaceInheritanceError> {
    let project_name = raw.name.clone();

    macro_rules! resolve {
        ($field:expr, $name:literal, $proj_fn:expr, $grp_fn:expr) => {{
            let (v, _) = resolve_field($field, $name, &project_name, workspace, $proj_fn, $grp_fn)?;
            v
        }};
    }

    macro_rules! resolve_opt {
        ($field:expr, $name:literal, $proj_fn:expr, $grp_fn:expr) => {{
            let (v, _) =
                resolve_optional_field($field, $name, &project_name, workspace, $proj_fn, $grp_fn)?;
            v
        }};
    }

    let version = resolve!(
        raw.version,
        "version",
        |ws| ws.project.as_ref().and_then(|p| p.version.as_deref()),
        |e| e.project.as_ref().and_then(|p| p.version.as_deref())
    );
    let publisher = resolve_opt!(
        raw.publisher,
        "publisher",
        |ws| ws.project.as_ref().and_then(|p| p.publisher.as_deref()),
        |e| e.project.as_ref().and_then(|p| p.publisher.as_deref())
    );
    let license = resolve_opt!(
        raw.license,
        "license",
        |ws| ws.project.as_ref().and_then(|p| p.license.as_deref()),
        |e| e.project.as_ref().and_then(|p| p.license.as_deref())
    );

    Ok(InterchangeProjectInfoRaw {
        name: raw.name,
        publisher,
        description: raw.description,
        version,
        license,
        maintainer: raw.maintainer,
        website: raw.website,
        topic: raw.topic,
        usage: raw.usage,
    })
}

/// Resolve the `metamodel` field of `.meta.json`.
///
/// `{ "workspace": true }` inherits from [`WorkspaceInfo::meta`]`.metamodel`;
/// `{ "workspace": "group" }` inherits from
/// [`WorkspaceGroupEntryRaw::meta`]`.metamodel`. A literal value or absent
/// field is passed through unchanged.
///
/// `project_name` is the owning project's name (from `.project.json`) and is
/// used only in error messages.
///
/// # Errors
///
/// Returns [`WorkspaceInheritanceError`] under the same conditions as
/// [`resolve_project_info`].
pub fn resolve_project_metadata(
    raw: crate::model::InterchangeProjectMetadataWithInheritRaw,
    workspace: &WorkspaceInfo,
    project_name: &str,
) -> Result<InterchangeProjectMetadataRaw, WorkspaceInheritanceError> {
    let (metamodel, _) = resolve_optional_field(
        raw.metamodel,
        "metamodel",
        project_name,
        workspace,
        |ws| {
            ws.meta
                .as_ref()
                .and_then(|m| m.metamodel.as_ref().map(|i| i.as_str()))
        },
        |e| e.meta.as_ref().and_then(|m| m.metamodel.as_deref()),
    )?;

    Ok(InterchangeProjectMetadataRaw {
        index: raw.index,
        created: raw.created,
        metamodel,
        includes_derived: raw.includes_derived,
        includes_implied: raw.includes_implied,
        checksum: raw.checksum,
    })
}

/// Convert `.project.json` inheritance fields to plain values when no workspace
/// is available.
///
/// Literal fields are passed through; any `{ "workspace": ... }` value causes a
/// [`WorkspaceInheritanceError::NoWorkspace`] error.
pub fn project_info_without_workspace(
    raw: crate::model::InterchangeProjectInfoWithInheritRaw,
) -> Result<InterchangeProjectInfoRaw, WorkspaceInheritanceError> {
    let project_name = raw.name.clone();

    fn no_ws<T>(
        field: WorkspaceInherit<T>,
        project_name: &str,
        _field_name: &'static str,
    ) -> Result<T, WorkspaceInheritanceError> {
        match field {
            WorkspaceInherit::Literal(v) => Ok(v),
            WorkspaceInherit::Preset { .. } => Err(WorkspaceInheritanceError::NoWorkspace {
                project: project_name.to_string(),
            }),
        }
    }

    fn no_ws_opt<T>(
        field: Option<WorkspaceInherit<T>>,
        project_name: &str,
    ) -> Result<Option<T>, WorkspaceInheritanceError> {
        field.map(|f| no_ws(f, project_name, "")).transpose()
    }

    Ok(InterchangeProjectInfoRaw {
        name: raw.name,
        publisher: no_ws_opt(raw.publisher, &project_name)?,
        description: raw.description,
        version: no_ws(raw.version, &project_name, "version")?,
        license: no_ws_opt(raw.license, &project_name)?,
        maintainer: raw.maintainer,
        website: raw.website,
        topic: raw.topic,
        usage: raw.usage,
    })
}

/// Convert `.meta.json` inheritance fields to plain values when no workspace
/// is available.
///
/// A literal or absent `metamodel` is passed through; `{ "workspace": ... }`
/// causes a [`WorkspaceInheritanceError::NoWorkspace`] error.
///
/// `project_name` is the owning project's name and is used only in error
/// messages.
pub fn project_metadata_without_workspace(
    raw: crate::model::InterchangeProjectMetadataWithInheritRaw,
    project_name: &str,
) -> Result<InterchangeProjectMetadataRaw, WorkspaceInheritanceError> {
    let metamodel = match raw.metamodel {
        None => None,
        Some(WorkspaceInherit::Literal(v)) => Some(v),
        Some(WorkspaceInherit::Preset { .. }) => {
            return Err(WorkspaceInheritanceError::NoWorkspace {
                project: project_name.to_string(),
            });
        }
    };

    Ok(InterchangeProjectMetadataRaw {
        index: raw.index,
        created: raw.created,
        metamodel,
        includes_derived: raw.includes_derived,
        includes_implied: raw.includes_implied,
        checksum: raw.checksum,
    })
}
