// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    model::InterchangeProjectChecksumRaw,
    project::{ProjectMut, ProjectOrIOError, utils::FsIoError},
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
}

impl<ProjectError> From<ProjectOrIOError<ProjectError>> for ExcludeError<ProjectError> {
    fn from(value: ProjectOrIOError<ProjectError>) -> Self {
        match value {
            ProjectOrIOError::Project(error) => ExcludeError::Project(error),
            ProjectOrIOError::Io(error) => ExcludeError::from(error),
        }
    }
}

pub fn do_exclude<Pr: ProjectMut, P: AsRef<Utf8UnixPath>, I: Iterator<Item = P>>(
    project: &mut Pr,
    paths: I,
) -> Result<Vec<SourceExclusionOutcome>, ExcludeError<Pr::Error>> {
    let mut meta = match project.get_meta().map_err(ProjectOrIOError::Project)? {
        Some(m) => m,
        None => return Err(ExcludeError::MissingMeta),
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

#[derive(Debug)]
pub struct SourceExclusionOutcome {
    pub removed_checksum: Option<InterchangeProjectChecksumRaw>,
    pub removed_symbols: Vec<String>,
}
