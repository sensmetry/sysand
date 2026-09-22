// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    model::InterchangeProjectChecksumRaw,
    project::{ProjectFormat, ProjectMut, ProjectOrIOError, utils::FsIoError},
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
    /// The manifest could not be read, or the edit was refused.
    ///
    /// `exclude_source` can fail four ways and only one of them is "not
    /// listed". The rest keep their own words: `sysand exclude
    /// README.md` says the entry is not a model file, which is a strange
    /// answer to "remove this" and the true one -- `sources` could never
    /// have held it.
    #[error(transparent)]
    Edit(crate::manifest::edit::EditError),
    #[error("failed to read the project's `model.toml`: {0}")]
    Manifest(Box<str>),
    /// The `root` is one of the `sources` by the Syside schema's rule, so
    /// removing it from `sources` would leave a manifest the next read
    /// refuses.
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
    // model-toml.md "Editing the manifest": "`sysand exclude <paths>`
    // removes it." `Both` falls through to the json arm, where
    // `get_meta` refuses it as the ambiguity it is.
    if project.format().map_err(ProjectOrIOError::Project)? == ProjectFormat::Toml {
        return exclude_toml(project, paths);
    }

    let Some(mut meta) = project.get_meta().map_err(ProjectOrIOError::Project)? else {
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
/// The outcome has no toml meaning -- there is no checksum or symbol
/// map to remove from -- so an empty one is returned per path, although
/// the json arm reads an empty outcome as "not found".
fn exclude_toml<Pr: ProjectMut, P: AsRef<Utf8UnixPath>, I: Iterator<Item = P>>(
    project: &mut Pr,
    paths: I,
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<Pr::Error>> {
    let mut document = project.manifest_document().map_err(|e| match e {
        crate::project::ManifestError::Project(e) => ExcludeError::Project(e),
        crate::project::ManifestError::NoTomlManifest => {
            ExcludeError::Manifest("this project has no `model.toml` manifest".into())
        }
    })?;

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
                // The one failure whose text the json arm already has.
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
        .map_err(|e| match e {
            crate::project::ManifestWriteError::Project(e) => ExcludeError::Project(e),
            crate::project::ManifestWriteError::Unsupported => {
                ExcludeError::Manifest("this project cannot store a `model.toml`".into())
            }
        })?;
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
