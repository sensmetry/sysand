// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use indexmap::IndexMap;

#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::KNOWN_METAMODELS;

/// Workspace-level defaults for inheritable `.project.json` fields.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjectDefaultsRaw {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// A named workspace group: project-level defaults and optional meta defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGroupEntryRaw {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<WorkspaceProjectDefaultsRaw>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<WorkspaceMetaRaw>,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjectInfoG<Iri> {
    pub path: String,
    pub iris: Vec<Iri>,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug, Default)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMetaG<Iri> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metamodel: Option<Iri>,
}

pub type WorkspaceMetaRaw = WorkspaceMetaG<String>;
pub type WorkspaceMeta = WorkspaceMetaG<Iri<String>>;

#[derive(Error, Debug)]
pub enum WorkspaceValidationError {
    #[error("failed to parse `{0}` as IRI: {1}")]
    InvalidIri(String, fluent_uri::ParseError),
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfoG<Iri> {
    pub projects: Vec<WorkspaceProjectInfoG<Iri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<WorkspaceMetaG<Iri>>,
    /// Workspace-level defaults for inheritable project fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<WorkspaceProjectDefaultsRaw>,
    /// Named project groups, each with their own project defaults and meta.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub groups: Option<IndexMap<String, WorkspaceGroupEntryRaw>>,
}

pub type WorkspaceInfoRaw = WorkspaceInfoG<String>;
pub type WorkspaceInfo = WorkspaceInfoG<Iri<String>>;
pub type WorkspaceProjectInfoRaw = WorkspaceProjectInfoG<String>;
pub type WorkspaceProjectInfo = WorkspaceProjectInfoG<Iri<String>>;

impl TryFrom<WorkspaceInfoRaw> for WorkspaceInfo {
    type Error = WorkspaceValidationError;

    fn try_from(value: WorkspaceInfoRaw) -> Result<Self, Self::Error> {
        let mut projects = Vec::with_capacity(value.projects.len());
        for project in value.projects {
            let mut iris = Vec::with_capacity(project.iris.len());
            for iri in project.iris {
                let iri = Iri::parse(iri)
                    .map_err(|(e, iri)| WorkspaceValidationError::InvalidIri(iri, e))?;
                iris.push(iri);
            }
            projects.push(WorkspaceProjectInfo {
                path: project.path,
                iris,
            });
        }

        let meta = value
            .meta
            .map(|raw_meta| {
                let metamodel = raw_meta
                    .metamodel
                    .map(|m| {
                        if !KNOWN_METAMODELS.contains(&m.as_str()) {
                            log::warn!("workspace uses an unknown metamodel `{m}`");
                        }
                        Iri::parse(m)
                            .map_err(|(e, iri)| WorkspaceValidationError::InvalidIri(iri, e))
                    })
                    .transpose()?;
                Ok(WorkspaceMeta { metamodel })
            })
            .transpose()?;

        // `project` and `groups` fields use only String-based types; pass through unchanged.
        Ok(Self {
            projects,
            meta,
            project: value.project,
            groups: value.groups,
        })
    }
}
