// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use semver::Version;

use crate::{
    model::{InterchangeProjectInfo, InterchangeProjectMetadata},
    project::{ProjectMut, memory::InMemoryProject},
};

#[cfg(feature = "filesystem")]
use crate::project::local_src::LocalSrcProject;
#[cfg(feature = "filesystem")]
use std::path::Path;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NewError<ProjectError: std::error::Error> {
    #[error("refusing to overwrite: {0}")]
    AlreadyExists(String),
    #[error("{0}")]
    SemVerError(semver::Error),
    #[error("{0}")]
    ProjectError(#[from] ProjectError),
}

pub fn do_new<S: ProjectMut>(
    name: String,
    version: String,
    storage: &mut S,
) -> Result<(), NewError<S::Error>> {
    let version = Version::parse(&version).map_err(NewError::SemVerError)?;

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");

    storage.put_project(
        &InterchangeProjectInfo {
            name,
            description: None,
            version,
            license: None,
            maintainer: vec![],
            topic: vec![],
            usage: vec![],
            website: None,
        }
        .into(),
        &InterchangeProjectMetadata {
            index: indexmap::IndexMap::new(),
            created: chrono::Utc::now(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
        .into(),
        false,
    )?;

    Ok(())
}

pub fn do_new_memory(
    name: String,
    version: String,
) -> Result<InMemoryProject, NewError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_new(name, version, &mut storage)?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_new_local_file<P: AsRef<Path>>(
    name: String,
    version: String,
    path: P,
) -> Result<LocalSrcProject, NewError<crate::project::local_src::LocalSrcError>> {
    let mut storage = LocalSrcProject {
        project_path: path.as_ref().to_path_buf(),
    };

    do_new(name, version, &mut storage)?;

    Ok(storage)
}
