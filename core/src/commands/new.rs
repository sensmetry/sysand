// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use semver::Version;
use spdx;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadata},
    project::{ProjectMut, memory::InMemoryProject},
};

#[cfg(feature = "filesystem")]
use crate::project::local_src::LocalSrcProject;
#[cfg(feature = "filesystem")]
use std::path::Path;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NewError<ProjectError: std::error::Error> {
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
}

pub fn do_new_ext<S: ProjectMut>(
    name: String,
    version: String,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    storage: &mut S,
) -> Result<(), NewError<S::Error>> {
    if !no_semver {
        Version::parse(&version).map_err(|e| NewError::SemVerParse(version.as_str().into(), e))?;
    }
    let license = if let Some(l) = license {
        if !no_spdx {
            spdx::Expression::parse(&l)
                .map_err(|e| NewError::SPDXLicenseParse(l.as_str().into(), e))?;
        }
        Some(l)
    } else {
        None
    };

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");

    storage.put_project(
        &InterchangeProjectInfoRaw {
            name,
            description: None,
            version,
            license,
            maintainer: vec![],
            topic: vec![],
            usage: vec![],
            website: None,
        },
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

pub fn do_new<S: ProjectMut>(
    name: String,
    version: String,
    license: Option<String>,
    storage: &mut S,
) -> Result<(), NewError<S::Error>> {
    do_new_ext(name, version, false, license, false, storage)
}

pub fn do_new_memory(
    name: String,
    version: String,
    license: Option<String>,
) -> Result<InMemoryProject, NewError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_new(name, version, license, &mut storage)?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_new_local_file<P: AsRef<Path>>(
    name: String,
    version: String,
    license: Option<String>,
    path: P,
) -> Result<LocalSrcProject, NewError<crate::project::local_src::LocalSrcError>> {
    let mut storage = LocalSrcProject {
        project_path: path.as_ref().to_path_buf(),
    };

    do_new(name, version, license, &mut storage)?;

    Ok(storage)
}
