// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectMut, ProjectRead, utils::FsIoError},
};

use thiserror::Error;

/// Trait to use as a bound for all errors exposed through public
/// crate interfaces. This makes it convenient to use anyhow::Error.
pub trait ErrorBound: std::error::Error + Send + Sync + 'static {}
impl<T> ErrorBound for T where T: std::error::Error + Send + Sync + 'static {}

#[derive(Error, Debug)]
pub enum CloneError<ProjectReadError: ErrorBound, EnvironmentWriteError: ErrorBound> {
    #[error("project read error: {0}")]
    ProjectRead(ProjectReadError),
    #[error("environment write error: {0}")]
    EnvWrite(EnvironmentWriteError),
    #[error("incomplete project: {0}")]
    IncompleteSource(&'static str),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl<ProjectReadError: ErrorBound, EnvironmentWriteError: ErrorBound> From<FsIoError>
    for CloneError<ProjectReadError, EnvironmentWriteError>
{
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

/// Copies the project from `from` to `to`. Returns project metadata
pub fn clone_project<P: ProjectRead, Q: ProjectMut>(
    from: &P,
    to: &mut Q,
    overwrite: bool,
) -> Result<
    (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw),
    CloneError<P::Error, Q::Error>,
> {
    match from.get_project().map_err(CloneError::ProjectRead)? {
        (None, None) => Err(CloneError::IncompleteSource(
            "missing `.project.json` and `.meta.json`",
        )),
        (None, _) => Err(CloneError::IncompleteSource("missing `.project.json`")),
        (_, None) => Err(CloneError::IncompleteSource("missing `.meta.json`")),
        (Some(info), Some(meta)) => {
            to.put_project(&info, &meta, overwrite)
                .map_err(CloneError::EnvWrite)?;

            for source_path in &meta.source_paths(true) {
                let mut source = from
                    .read_source(source_path)
                    .map_err(CloneError::ProjectRead)?;
                to.write_source(source_path, &mut source, overwrite)
                    .map_err(CloneError::EnvWrite)?;
            }
            Ok((info, meta))
        }
    }
}
