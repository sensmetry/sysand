// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::project::{ProjectMut, ProjectOrIOError, SourceExclusionOutcome};

#[derive(Error, Debug)]
pub enum ExcludeError<ProjectError> {
    #[error("{0}")]
    ProjectError(ProjectError),
    #[error("{0}")]
    IOError(std::io::Error),
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for ExcludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::ProjectError(error) => ExcludeError::ProjectError(error),
            ProjectOrIOError::IOError(error) => ExcludeError::IOError(error),
        }
    }
}

pub fn do_exclude<Pr: ProjectMut, P: AsRef<Utf8UnixPath>>(
    project: &mut Pr,
    path: P,
) -> Result<SourceExclusionOutcome, ExcludeError<Pr::Error>> {
    Ok(project.exclude_source(path)?)
}
