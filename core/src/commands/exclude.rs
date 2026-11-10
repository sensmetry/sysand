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
    Manifest(crate::project::ManifestError<ProjectError>),
    #[error(transparent)]
    ManifestWrite(crate::project::ManifestWriteError<ProjectError>),
    /// Removing the `root` from `sources` would leave a manifest that
    /// cannot be read.
    #[error("`{0}` is the project's `root`; set another `root` before excluding it")]
    RootExcluded(Box<str>),
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
    let mut document = project
        .manifest_document()
        .map_err(ExcludeError::Manifest)?;

    let mut outcomes = Vec::new();
    for path in paths {
        let path = path.as_ref();
        let excluding = "Excluding";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{excluding:>12}{header:#} file: `{path}`");

        let root = document
            .get("project")
            .and_then(|project| project.get("root"))
            .and_then(|root| root.as_str());
        let normalized = crate::utils::normalize_relative(path.as_str())
            .map_or_else(|_| path.as_str().to_owned(), |p| p.as_str().to_owned());
        if root.is_some_and(|root| root == path.as_str() || root == normalized) {
            return Err(ExcludeError::RootExcluded(path.as_str().into()));
        }
        crate::manifest::edit::exclude_source(&mut document, path.as_str()).map_err(|e| {
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

    project
        .put_manifest_document(&document, true)
        .map_err(ExcludeError::ManifestWrite)?;
    Ok(outcomes)
}

#[derive(Debug)]
pub struct SourceExclusionOutcome {
    pub removed_checksum: Option<InterchangeProjectChecksumRaw>,
    pub removed_symbols: Vec<String>,
}

#[cfg(test)]
#[path = "./exclude_tests.rs"]
mod tests;
