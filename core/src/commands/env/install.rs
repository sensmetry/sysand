// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    env::{
        PutProjectError, ReadEnvironment, WriteEnvironment,
        utils::{CloneError, clone_project},
    },
    project::ProjectRead,
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

#[allow(clippy::type_complexity)]
pub fn do_env_install_project<
    S: AsRef<str>,
    P: ProjectRead,
    E: WriteEnvironment + ReadEnvironment,
>(
    uri: S,
    version: &str,
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
    check_install(&uri, version, env, allow_overwrite, allow_multiple)?;

    let installing = "Installing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{installing:>12}{header:#} `{}` {version}",
        uri.as_ref(),
    );

    env.put_project(uri, version, |p| {
        clone_project(storage, p, true).map(|_| ())
    })
    .map_err(EnvInstallError::Installation)?;

    Ok(())
}
