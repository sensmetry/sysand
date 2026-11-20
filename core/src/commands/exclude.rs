// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

#[cfg(test)]
use crate::{commands::edit_testing::edit_in_memory, project::memory::InMemoryProject};
#[cfg(feature = "filesystem")]
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::{
    context::ManifestEdit,
    manifest::{
        edit::{self, DocumentMut},
        project::ResolvedProject,
    },
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::local_src::{LocalSrcError, LocalSrcProject},
};
use crate::{
    manifest::edit::EditError,
    model::InterchangeProjectChecksumRaw,
    project::{ProjectOrIOError, utils::FsIoError},
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
    Edit(EditError),
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

/// Excluding source files from the current project, and reporting what was
/// removed for each.
///
/// A `model.toml` project has no checksum or symbol map to remove from, so
/// an empty outcome is reported per path.
#[cfg(feature = "filesystem")]
#[derive(Debug)]
pub struct ExcludeFiles {
    pub paths: Vec<Utf8UnixPathBuf>,
}

#[cfg(feature = "filesystem")]
impl ManifestEdit for ExcludeFiles {
    type Output = Vec<SourceExclusionOutcome>;
    type Error = ExcludeError<LocalSrcError>;

    fn toml(
        self,
        document: &mut DocumentMut,
        project: &ResolvedProject,
    ) -> Result<Self::Output, Self::Error> {
        exclude_toml(document, project, &self.paths)
    }

    fn json(
        self,
        _project: &LocalSrcProject,
        _info: &mut InterchangeProjectInfoRaw,
        meta: &mut InterchangeProjectMetadataRaw,
    ) -> Result<Self::Output, Self::Error> {
        exclude_json(meta, &self.paths)
    }
}

/// `exclude` on a json project.
#[cfg(feature = "filesystem")]
fn exclude_json<E>(
    meta: &mut InterchangeProjectMetadataRaw,
    paths: &[Utf8UnixPathBuf],
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<E>> {
    let mut outcomes = Vec::new();
    for path in paths {
        let path = path.as_path();
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
    Ok(outcomes)
}

/// `exclude` on a `model.toml` project: remove each path from `sources`.
#[cfg(feature = "filesystem")]
fn exclude_toml<E>(
    document: &mut DocumentMut,
    project: &ResolvedProject,
    paths: &[Utf8UnixPathBuf],
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<E>> {
    if project.metamodel.family == crate::model::MetamodelFamily::Syside {
        return Err(ExcludeError::RootOnly);
    }
    let mut outcomes = Vec::new();
    for path in paths {
        let path = path.as_path();
        let excluding = "Excluding";
        let header = crate::style::get_style_config().header;
        log::info!("{header}{excluding:>12}{header:#} file: `{path}`");

        edit::exclude_source(document, path.as_str()).map_err(ExcludeError::Edit)?;
        outcomes.push(SourceExclusionOutcome {
            removed_checksum: None,
            removed_symbols: Vec::new(),
        });
    }
    Ok(outcomes)
}

#[derive(Debug)]
pub struct SourceExclusionOutcome {
    pub removed_checksum: Option<InterchangeProjectChecksumRaw>,
    pub removed_symbols: Vec<String>,
}

/// [`ExcludeFiles`] on an in-memory project.
#[cfg(test)]
pub(crate) fn exclude_in_memory<P: AsRef<typed_path::Utf8UnixPath>>(
    project: &mut InMemoryProject,
    paths: impl Iterator<Item = P>,
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<std::convert::Infallible>> {
    let paths: Vec<Utf8UnixPathBuf> = paths
        .map(|path| Utf8UnixPathBuf::from(path.as_ref().as_str()))
        .collect();
    edit_in_memory(
        project,
        |document, resolved| exclude_toml(document, resolved, &paths),
        |_, meta| exclude_json(meta, &paths),
    )
}

#[cfg(test)]
#[path = "./exclude_tests.rs"]
mod tests;
