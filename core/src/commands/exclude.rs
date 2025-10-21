// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::project::{ProjectMut, ProjectOrIOError, SourceExclusionOutcome, utils::FsIoError};

#[derive(Error, Debug)]
pub enum ExcludeError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("could not find {0} in project metadata")]
    SourceNotFound(String),
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for ExcludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::ProjectError(error) => ExcludeError::Project(error),
            ProjectOrIOError::Io(error) => ExcludeError::from(error),
        }
    }
}

pub fn do_exclude<Pr: ProjectMut, P: AsRef<Utf8UnixPath>>(
    project: &mut Pr,
    path: P,
) -> Result<SourceExclusionOutcome, ExcludeError<Pr::Error>> {
    let excluding = "Excluding";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{excluding:>12}{header:#} file: {}", path.as_ref(),);

    let outcome = project.exclude_source(&path)?;

    if outcome.removed_checksum.is_some() {
        Ok(outcome)
    } else {
        Err(ExcludeError::SourceNotFound(path.as_ref().to_string()))
    }

    // Ok(project.exclude_source(path)?)
}
