// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    env::{ReadEnvironment, WriteEnvironment, utils::clone_project},
    project::ProjectRead,
};

#[derive(Error, Debug)]
enum CheckInstallError<EnvReadError, ProjectReadError> {
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
}

fn check_install<S: AsRef<str>, P: ProjectRead, E: ReadEnvironment>(
    uri: S,
    storage: &P,
    env: &E,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<(), CheckInstallError<E::ReadError, P::Error>> {
    if allow_overwrite && allow_multiple {
        return Ok(());
    }

    if !allow_overwrite && !allow_multiple {
        if env.has(&uri).map_err(CheckInstallError::EnvRead)? {
            return Err(CheckInstallError::AlreadyInstalled(uri.as_ref().into()));
        }
        return Ok(());
    }

    if let Some(version) = storage.version().map_err(CheckInstallError::ProjectRead)? {
        if env.has(&uri).map_err(CheckInstallError::EnvRead)? {
            let version_present = env
                .has_version(&uri, &version)
                .map_err(CheckInstallError::EnvRead)?;

            if !allow_overwrite && version_present {
                return Err(CheckInstallError::AlreadyInstalledVersion(
                    uri.as_ref().into(),
                    version,
                ));
            }
            if !allow_multiple && !version_present {
                return Err(CheckInstallError::AlreadyInstalledUnknownVersion(
                    uri.as_ref().into(),
                ));
            }
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
    crate::env::PutProjectError<
        EnvWriteError,
        crate::env::utils::CloneError<ProjectReadError, ProjectWriteError>,
    >;

impl<EnvReadError, ProjectReadError, I> From<CheckInstallError<EnvReadError, ProjectReadError>>
    for EnvInstallError<EnvReadError, ProjectReadError, I>
{
    fn from(value: CheckInstallError<EnvReadError, ProjectReadError>) -> Self {
        match value {
            CheckInstallError::AlreadyInstalled(s) => EnvInstallError::AlreadyInstalled(s),
            CheckInstallError::EnvRead(e) => EnvInstallError::EnvRead(e),
            CheckInstallError::ProjectRead(e) => EnvInstallError::ProjectRead(e),
            CheckInstallError::AlreadyInstalledVersion(iri, version) => {
                Self::AlreadyInstalledVersion(iri, version)
            }
            CheckInstallError::AlreadyInstalledUnknownVersion(iri) => {
                Self::AlreadyInstalledUnknownVersion(iri)
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn do_env_install_project<
    S: AsRef<str>,
    P: ProjectRead,
    E: WriteEnvironment + ReadEnvironment,
>(
    uri: S,
    storage: &P,
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
    check_install(&uri, storage, env, allow_overwrite, allow_multiple)?;

    let version = storage
        .version()
        .map_err(EnvInstallError::ProjectRead)?
        .ok_or(EnvInstallError::MissingSpec)?;

    let installing = "Installing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{installing:>12}{header:#} `{}` {version}",
        uri.as_ref(),
    );

    // TODO: print version(s) being installed
    env.put_project(uri, version, |p| {
        clone_project(storage, p, true).map(|_| ())
    })
    .map_err(EnvInstallError::Installation)?;

    Ok(())
}
