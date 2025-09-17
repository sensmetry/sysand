// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    project::{ProjectMut, ProjectOrIOError},
    symbols::{ExtractError, Language},
};

#[derive(Error, Debug)]
pub enum IncludeError<ProjectError> {
    #[error("{0}")]
    ProjectError(ProjectError),
    #[error("{0}")]
    IOError(#[from] std::io::Error),
    #[error("{0}")]
    ExtractError(#[from] ExtractError),
    #[error("unknown file format {0}")]
    UnknownFormat(String),
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for IncludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::ProjectError(error) => IncludeError::ProjectError(error),
            ProjectOrIOError::IOError(error) => IncludeError::IOError(error),
        }
    }
}

// TODO: Add some option to make the file format explicit
pub fn do_include<Pr: ProjectMut, P: AsRef<Utf8UnixPath>>(
    project: &mut Pr,
    path: P,
    compute_checksum: bool,
    index_symbols: bool,
    force_format: Option<Language>,
) -> Result<(), IncludeError<Pr::Error>> {
    project.include_source(&path, compute_checksum, true)?;

    if index_symbols {
        match force_format.or_else(|| Language::guess_from_path(&path)) {
            Some(Language::SysML) => {
                let new_symbols = crate::symbols::top_level_sysml(
                    project
                        .read_source(&path)
                        .map_err(IncludeError::ProjectError)?,
                )?;

                project.merge_index(new_symbols.into_iter().map(|x| (x, path.as_ref())), true)?;
            }
            _ => {
                return Err(IncludeError::UnknownFormat(format!(
                    "cannot guess format for {}, only sysml supported",
                    path.as_ref()
                )));
            }
        }
    }
    Ok(())
}
