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

const DEFAULT_PUBLISHER: &str = "untitled";

#[derive(Error, Debug)]
pub enum InitError<ProjectError: ErrorBound> {
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
}

pub fn do_init_ext<P: ProjectMut>(
    name: String,
    publisher: Option<String>,
    version: String,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    if !no_semver {
        Version::parse(&version).map_err(|e| InitError::SemVerParse(version.as_str().into(), e))?;
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
    let publisher = publisher.unwrap_or_else(|| String::from(DEFAULT_PUBLISHER));

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} interchange project `{name}`");

    storage.put_project(
        &InterchangeProjectInfoRaw {
            name: name.to_owned(),
            publisher: Some(publisher),
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

pub fn do_init<P: ProjectMut>(
    name: String,
    publisher: Option<String>,
    version: String,
    license: Option<String>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    do_init_ext(name, publisher, version, false, license, false, storage)
}

pub fn do_init_memory<N: AsRef<str>, P: AsRef<str>, V: AsRef<str>>(
    name: N,
    publisher: Option<P>,
    version: V,
    license: Option<String>,
) -> Result<InMemoryProject, InitError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_init(
        name.as_ref().to_owned(),
        publisher.map(|p| String::from(p.as_ref())),
        version.as_ref().to_owned(),
        license,
        &mut storage,
    )?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_init_local_file(
    name: String,
    publisher: Option<String>,
    version: String,
    license: Option<String>,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let mut storage = LocalSrcProject {
        nominal_path: None,
        project_path: path,
    };

    do_init(name, publisher, version, license, &mut storage)?;

    Ok(storage)
}
