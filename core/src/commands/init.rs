// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use semver::Version;
use spdx;

use crate::{
    env::utils::ErrorBound,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadata},
    project::{ProjectMut, memory::InMemoryProject},
};

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InitError<ProjectError: ErrorBound> {
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
}

pub fn do_init_ext<N: AsRef<str>, V: AsRef<str>, P: ProjectMut>(
    name: N,
    version: V,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    let name = name.as_ref();
    let version = version.as_ref();
    if !no_semver {
        Version::parse(version).map_err(|e| InitError::SemVerParse(version.into(), e))?;
    }
    let license = if let Some(l) = license {
        if !no_spdx {
            spdx::Expression::parse(&l)
                .map_err(|e| InitError::SPDXLicenseParse(l.as_str().into(), e))?;
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
            name: name.to_owned(),
            description: None,
            version: version.to_owned(),
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

pub fn do_init<N: AsRef<str>, V: AsRef<str>, P: ProjectMut>(
    name: N,
    version: V,
    license: Option<String>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    do_init_ext(name, version, false, license, false, storage)
}

pub fn do_init_memory<N: AsRef<str>, V: AsRef<str>>(
    name: N,
    version: V,
    license: Option<String>,
) -> Result<InMemoryProject, InitError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_init(name, version, license, &mut storage)?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_init_local_file<N: AsRef<str>, V: AsRef<str>>(
    name: N,
    version: V,
    license: Option<String>,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let mut storage = LocalSrcProject {
        nominal_path: None,
        project_path: path,
    };

    do_init(name, version, license, &mut storage)?;

    Ok(storage)
}
