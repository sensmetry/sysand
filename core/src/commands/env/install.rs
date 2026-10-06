// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::{Utf8Path, Utf8PathBuf};
#[cfg(feature = "filesystem")]
use fluent_uri::Iri;
use thiserror::Error;

use crate::{
    env::{
        PutProjectError, ReadEnvironment, WriteEnvironment,
        utils::{CloneError, clone_project},
    },
    project::{ProjectChecksum, ProjectRead},
};
#[cfg(feature = "filesystem")]
use crate::{
    project::{
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        utils::{FsIoError, Identifier, wrapfs},
    },
    resolve::file::{FileResolverProject, FileResolverProjectError},
};

#[derive(Error, Debug)]
enum CheckInstallError<EnvReadError> {
    #[error("project with IRI `{0}` is already installed")]
    AlreadyInstalled(Box<str>),
    #[error("project with IRI `{0}` already has version `{1}` installed")]
    AlreadyInstalledVersion(Box<str>, String),
    #[error("unknown version of project with IRI `{0}` is already installed")]
    AlreadyInstalledUnknownVersion(Box<str>),
    #[error("environment read error: {0}")]
    EnvRead(EnvReadError),
}

fn check_install<S: AsRef<str>, E: ReadEnvironment>(
    uri: S,
    version: &str,
    env: &E,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<(), CheckInstallError<E::ReadError>> {
    if allow_overwrite && allow_multiple {
        return Ok(());
    }
    let project_present = env.has(&uri).map_err(CheckInstallError::EnvRead)?;

    if !allow_overwrite && !allow_multiple {
        if project_present {
            return Err(CheckInstallError::AlreadyInstalled(uri.as_ref().into()));
        }
        return Ok(());
    }

    if project_present {
        let version_present = env
            .has_version(&uri, version)
            .map_err(CheckInstallError::EnvRead)?;

        if !allow_overwrite && version_present {
            return Err(CheckInstallError::AlreadyInstalledVersion(
                uri.as_ref().into(),
                version.to_owned(),
            ));
        }
        if !allow_multiple && !version_present {
            return Err(CheckInstallError::AlreadyInstalledUnknownVersion(
                uri.as_ref().into(),
            ));
        }
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum EnvInstallError<EnvReadError, ProjectReadError, InstallationError> {
    #[error("project with IRI `{0}` is already installed")]
    AlreadyInstalled(Box<str>),
    #[error("project with IRI `{0}` already has version `{1}` installed")]
    AlreadyInstalledVersion(Box<str>, String),
    #[error("unknown version of project with IRI `{0}` is already installed")]
    AlreadyInstalledUnknownVersion(Box<str>),
    #[error("environment read error: {0}")]
    EnvRead(EnvReadError),
    #[error("project read error: {0}")]
    ProjectRead(ProjectReadError),
    #[error("missing spec error")]
    MissingSpec,
    #[error("project installation error: {0}")]
    Installation(InstallationError),
}

type InstallationError<EnvWriteError, ProjectReadError, ProjectWriteError> =
    PutProjectError<EnvWriteError, CloneError<ProjectReadError, ProjectWriteError>>;

/// The error of [`do_env_install_project`] installing `P` into `E`
pub type EnvInstallProjectError<E, P> = EnvInstallError<
    <E as ReadEnvironment>::ReadError,
    <P as ProjectRead>::Error,
    InstallationError<
        <E as WriteEnvironment>::WriteError,
        <P as ProjectRead>::Error,
        <<E as WriteEnvironment>::InterchangeProjectMut as ProjectRead>::Error,
    >,
>;

impl<EnvReadError, ProjectReadError, I> From<CheckInstallError<EnvReadError>>
    for EnvInstallError<EnvReadError, ProjectReadError, I>
{
    fn from(value: CheckInstallError<EnvReadError>) -> Self {
        match value {
            CheckInstallError::AlreadyInstalled(s) => Self::AlreadyInstalled(s),
            CheckInstallError::EnvRead(e) => Self::EnvRead(e),
            CheckInstallError::AlreadyInstalledVersion(iri, version) => {
                Self::AlreadyInstalledVersion(iri, version)
            }
            CheckInstallError::AlreadyInstalledUnknownVersion(iri) => {
                Self::AlreadyInstalledUnknownVersion(iri)
            }
        }
    }
}

pub fn do_env_install_project<
    S: AsRef<str>,
    P: ProjectRead,
    E: WriteEnvironment + ReadEnvironment,
>(
    uri: S,
    version: &str,
    storage: &P,
    checksum: Option<ProjectChecksum>,
    env: &mut E,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<
    (),
    EnvInstallError<
        E::ReadError,
        P::Error,
        InstallationError<
            E::WriteError,
            P::Error,
            <E::InterchangeProjectMut as ProjectRead>::Error,
        >,
    >,
> {
    check_install(&uri, version, env, allow_overwrite, allow_multiple)?;

    let installing = "Installing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{installing:>12}{header:#} `{}` {version}",
        uri.as_ref(),
    );

    env.put_project(uri, version, checksum, |p| {
        clone_project(storage, p, true).map(|_| ())
    })
    .map_err(EnvInstallError::Installation)?;

    Ok(())
}

#[cfg(feature = "filesystem")]
#[derive(Error, Debug)]
pub enum EnvInstallPathError<InstallError> {
    #[error("invalid IRI `{0}`")]
    IriParse(Box<str>, #[source] fluent_uri::ParseError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("unable to find project at `{0}`")]
    NotFound(Utf8PathBuf),
    #[error("project at `{0}` lacks project information")]
    MissingInfo(Utf8PathBuf),
    #[error(transparent)]
    ProjectRead(#[from] FileResolverProjectError),
    #[error(transparent)]
    Installation(InstallError),
}

/// Same as [`do_env_install_project`], but installs the project at `location`
/// (a KPAR or a project directory), with the version and checksum it has
#[cfg(feature = "filesystem")]
pub fn do_env_install_path<E: WriteEnvironment + ReadEnvironment>(
    identifier: &Identifier,
    location: &Utf8Path,
    env: &mut E,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<(), EnvInstallPathError<EnvInstallProjectError<E, FileResolverProject>>> {
    let metadata = wrapfs::metadata(location)?;
    let project = if metadata.is_file() {
        FileResolverProject::LocalKParProject(LocalKParProject::new_access(
            location,
            KparInnerPath::Guess,
            None,
        ))
    } else if metadata.is_dir() {
        FileResolverProject::LocalSrcProject(LocalSrcProject::new_access(location, None))
    } else {
        return Err(EnvInstallPathError::NotFound(location.to_owned()));
    };

    let Some(version) = project.version()? else {
        return Err(EnvInstallPathError::MissingInfo(location.to_owned()));
    };
    let checksum = project.checksum_canonical_variant()?;
    do_env_install_project(
        identifier,
        &version,
        &project,
        Some(checksum),
        env,
        allow_overwrite,
        allow_multiple,
    )
    .map_err(EnvInstallPathError::Installation)
}

/// Same as [`do_env_install_path`], but takes an unparsed IRI
#[cfg(feature = "filesystem")]
pub fn do_env_install_path_parse<E: WriteEnvironment + ReadEnvironment>(
    iri: String,
    location: &Utf8Path,
    env: &mut E,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<(), EnvInstallPathError<EnvInstallProjectError<E, FileResolverProject>>> {
    let iri = Iri::parse(iri).map_err(|(e, iri)| EnvInstallPathError::IriParse(iri.into(), e))?;
    do_env_install_path(
        &Identifier::from_iri_owned(iri),
        location,
        env,
        allow_overwrite,
        allow_multiple,
    )
}
