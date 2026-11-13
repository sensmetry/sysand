// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    model::InterchangeProjectChecksumRaw,
    project::{ProjectDefinition, ProjectFormat, ProjectMut, ProjectOrIOError, utils::FsIoError},
};

#[derive(Error, Debug)]
pub enum ExcludeError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("could not find file `{0}` in project metadata")]
    SourceNotFound(Box<str>),
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
    /// The manifest edit was refused.
    #[error(transparent)]
    Edit(crate::manifest::edit::EditError),
    #[error(transparent)]
    Manifest(#[from] crate::project::ManifestError<ProjectError>),
    #[error(transparent)]
    ManifestWrite(#[from] crate::project::ManifestWriteError<ProjectError>),
    /// A Syside-metamodel `model.toml` has no `sources` to remove from.
    #[error("a Syside-metamodel project lists no `sources`; its files are reached from its `root`")]
    RootOnly,
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for ExcludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::Project(error) => Self::Project(error),
            ProjectOrIOError::Io(error) => Self::from(error),
        }
    }
}

pub fn do_exclude<Pr: ProjectMut, P: AsRef<Utf8UnixPath>, I: Iterator<Item = P>>(
    project: &mut Pr,
    paths: I,
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<Pr::Error>> {
    // `Both` falls through to the json arm, which refuses it as ambiguous.
    if project.format().map_err(ProjectOrIOError::Project)? == ProjectFormat::Toml {
        return exclude_toml(project, paths);
    }

    let Some(ProjectDefinition::Json { info: _, mut meta }) =
        project.get_project().map_err(ProjectOrIOError::Project)?
    else {
        return Err(ExcludeError::MissingMeta);
    };

    let mut outcomes = Vec::new();
    for path in paths {
        let path = path.as_ref();
        let excluding = "Excluding";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{excluding:>12}{header:#} file: `{path}`");

        let removed_checksum = meta.remove_checksum(&path);
        let removed_symbols = meta.remove_index(&path);
        let outcome = SourceExclusionOutcome {
            removed_checksum,
            removed_symbols,
        };

        if outcome.removed_checksum.is_none() && outcome.removed_symbols.is_empty() {
            return Err(ExcludeError::SourceNotFound(path.as_str().into()));
        }
        outcomes.push(outcome);
    }
    project
        .put_meta(&meta, true)
        .map_err(ProjectOrIOError::Project)?;
    Ok(outcomes)
}

/// `exclude` on a `model.toml` project: remove each path from `sources`.
///
/// There is no checksum or symbol map to remove from, so an empty outcome is
/// returned per path.
fn exclude_toml<Pr: ProjectMut, P: AsRef<Utf8UnixPath>, I: Iterator<Item = P>>(
    project: &mut Pr,
    paths: I,
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<Pr::Error>> {
    crate::project::edit_manifest_document(project, |project, document| {
        if crate::include::project_metamodel(project, document)
            .is_some_and(|kind| kind.family == crate::model::MetamodelFamily::Syside)
        {
            return Err(ExcludeError::RootOnly);
        }
        let mut outcomes = Vec::new();
        for path in paths {
            let path = path.as_ref();
            let excluding = "Excluding";
            let header = crate::style::get_style_config().header;
            log::info!("{header}{excluding:>12}{header:#} file: `{path}`");

            crate::manifest::edit::exclude_source(document, path.as_str()).map_err(|e| {
                match e {
                    // Only this failure has json-arm wording.
                    crate::manifest::edit::EditError::SourceNotListed(_) => {
                        ExcludeError::SourceNotFound(path.as_str().into())
                    }
                    other => ExcludeError::Edit(other),
                }
            })?;
            outcomes.push(SourceExclusionOutcome {
                removed_checksum: None,
                removed_symbols: Vec::new(),
            });
        }
        Ok(outcomes)
    })
}

#[derive(Debug)]
pub struct SourceExclusionOutcome {
    pub removed_checksum: Option<InterchangeProjectChecksumRaw>,
    pub removed_symbols: Vec<String>,
}

#[cfg(test)]
#[path = "./exclude_tests.rs"]
mod tests;
