// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use fluent_uri::Iri;
use semver::Version;
use spdx;

use crate::{
    env::utils::ErrorBound,
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadata, ProjectFieldError, ProjectName,
        ProjectPublisher,
    },
    project::{ProjectMut, memory::InMemoryProject},
};

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InitError<ProjectError: ErrorBound> {
    #[error("invalid project name `{0}`: {1}")]
    NameParse(String, ProjectFieldError),
    #[error("invalid project publisher `{0}`: {1}")]
    PublisherParse(String, ProjectFieldError),
    #[error("failed to parse `{0}` as a Semantic Version: {1}")]
    SemVerParse(Box<str>, semver::Error),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("failed to parse `{0}` as an SPDX license expression:\n{1}")]
    SPDXLicenseParse(Box<str>, spdx::error::ParseError),
    #[error("invalid metamodel `{0}`: {1}")]
    MetamodelParse(Box<str>, fluent_uri::ParseError),
}

pub fn do_init<P: ProjectMut>(
    name: ProjectName,
    publisher: ProjectPublisher,
    version: Version,
    license: Option<spdx::Expression>,
    metamodel: Option<Iri<String>>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{creating:>12}{header:#} interchange project `{}`",
        name.as_str()
    );

    storage.put_project(
        &InterchangeProjectInfoRaw {
            name: name.into_string(),
            publisher: Some(publisher.into_string()),
            description: None,
            version: version.to_string(),
            license: license.map(|l| l.to_string()),
            maintainer: vec![],
            topic: vec![],
            usage: vec![],
            website: None,
        },
        &InterchangeProjectMetadata {
            index: indexmap::IndexMap::new(),
            created: jiff::Timestamp::now(),
            metamodel,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }
        .into(),
        false,
    )?;

    Ok(())
}

/// Same as `do_init`, but takes unparsed values
pub fn do_init_parse<P: ProjectMut>(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<String>,
    storage: &mut P,
) -> Result<(), InitError<P::Error>> {
    let name = ProjectName::parse(name).map_err(|(name, e)| InitError::NameParse(name, e))?;
    let publisher = ProjectPublisher::parse(publisher)
        .map_err(|(publisher, e)| InitError::PublisherParse(publisher, e))?;
    let version =
        Version::parse(&version).map_err(|e| InitError::SemVerParse(version.as_str().into(), e))?;
    let license = if let Some(l) = license {
        let l = spdx::Expression::parse(&l)
            .map_err(|e| InitError::SPDXLicenseParse(l.as_str().into(), e))?;
        Some(l)
    } else {
        None
    };
    let metamodel = metamodel
        .map(Iri::parse)
        .transpose()
        .map_err(|(e, m)| InitError::MetamodelParse(m.into(), e))?;
    do_init(name, publisher, version, license, metamodel, storage)
}

pub fn do_init_memory<N: AsRef<str>, P: AsRef<str>, V: AsRef<str>>(
    name: N,
    publisher: P,
    version: V,
    license: Option<String>,
) -> Result<InMemoryProject, InitError<crate::project::memory::InMemoryError>> {
    let mut storage = InMemoryProject::default();

    do_init_parse(
        name.as_ref().to_owned(),
        publisher.as_ref().to_owned(),
        version.as_ref().to_owned(),
        license,
        None,
        &mut storage,
    )?;

    Ok(storage)
}

#[cfg(feature = "filesystem")]
pub fn do_init_local_file(
    name: String,
    publisher: String,
    version: String,
    license: Option<String>,
    metamodel: Option<String>,
    path: Utf8PathBuf,
) -> Result<LocalSrcProject, InitError<LocalSrcError>> {
    let mut storage = LocalSrcProject::new_access(path, None);
    do_init_parse(name, publisher, version, license, metamodel, &mut storage)?;

    Ok(storage)
}
