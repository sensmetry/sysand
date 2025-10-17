// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    env::{ReadEnvironment, WriteEnvironment, utils::clone_project},
    project::ProjectRead,
};

#[derive(Error, Debug)]
enum CheckInstallError<EnvReadError, ProjectReadError> {
    #[error("{0} already installed")]
    AlreadyInstalled(String),
    #[error("env read error")]
    EnvReadError(EnvReadError),
    #[error("project read error: {0}")]
    ProjectReadError(ProjectReadError),
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
        if env.has(&uri).map_err(CheckInstallError::EnvReadError)? {
            return Err(CheckInstallError::AlreadyInstalled(
                uri.as_ref().to_string(),
            ));
        }
        return Ok(());
    }

    if let Some(version) = storage
        .version()
        .map_err(CheckInstallError::ProjectReadError)?
    {
        if env.has(&uri).map_err(CheckInstallError::EnvReadError)? {
            let version_present = env
                .has_version(&uri, &version)
                .map_err(CheckInstallError::EnvReadError)?;

            if !allow_overwrite && version_present {
                return Err(CheckInstallError::AlreadyInstalled(format!(
                    "{} {}",
                    uri.as_ref(),
                    version
                )));
            }
            if !allow_multiple && !version_present {
                return Err(CheckInstallError::AlreadyInstalled(format!(
                    "other version of {}",
                    uri.as_ref(),
                )));
            }
        }
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum EnvInstallError<EnvReadError, ProjectReadError, InstallationError> {
    #[error("{0} already installed: {0}")]
    AlreadyInstalled(String),
    #[error("env read error: {0}")]
    EnvReadError(EnvReadError),
    #[error("project read error: {0}")]
    ProjectReadError(ProjectReadError),
    #[error("missing spec error")]
    MissingSpec,
    #[error("installation error: {0}")]
    InstallationError(InstallationError),
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
            CheckInstallError::EnvReadError(e) => EnvInstallError::EnvReadError(e),
            CheckInstallError::ProjectReadError(e) => EnvInstallError::ProjectReadError(e),
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
        .map_err(EnvInstallError::ProjectReadError)?
        .ok_or(EnvInstallError::MissingSpec)?;

    let installing = "Installing";
    let header = crate::style::get_style_config().header;
    log::info!(
        "{header}{installing:>12}{header:#} {} {version}",
        uri.as_ref(),
    );

    env.put_project(uri, version, |p| clone_project(storage, p, true))
        .map_err(EnvInstallError::InstallationError)?;

    Ok(())
}
