// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use semver::VersionReq;
use sysand_core::{
    env::local_directory::metadata::EnvMetadataError,
    info::InfoProjectError,
    lock::LockfileError,
    model::{IdentifierError, ProjectFieldError},
    resolve::file::FileResolverProjectError,
};
use thiserror::Error;

use crate::hints::Hint;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("invalid directory: {0}")]
    InvalidDirectory(String),
    #[error("cannot use the directory name `{0}` as the project name; use `--name` to set it")]
    InvalidDirectoryName(String, #[source] ProjectFieldError),
    #[error(
        "cannot use the directory name `{}` as the project name; use `--name` to set it",
        .0.value
    )]
    InvalidDirectoryIdentifier(#[source] IdentifierError),
    #[error("invalid project with IRI `{iri}`")]
    InvalidProject {
        iri: String,
        #[source]
        source: InfoProjectError<FileResolverProjectError>,
    },
    #[error("unable to find interchange project `{0}`")]
    MissingProject(String),
    #[error("unable to find interchange project `{0}` with version matching {1}")]
    MissingProjectVersion(String, VersionReq),
    #[error(
        "interchange project `{0}` has no released version,\n\
        only pre-releases ({1}); pass the version to use one of them"
    )]
    OnlyPrereleaseVersions(String, String),
    #[error("unable to find interchange project in current directory")]
    MissingProjectCurrentDir,
    #[error(
        "project `{0}` does not have a publisher,\n\
        which is required to add it as a path usage"
    )]
    MissingPublisherForUsage(String),
}

/// Why a command failed, with the hint for the frontend to print beneath
/// it, if there is one.
///
/// Any error converts into it, without a hint; [`Failure::lockfile`] and
/// [`Failure::environment`] add the hint their error calls for. Not
/// convertible into [`anyhow::Error`] by `From`, so that the conversions
/// from it can be that general: see [`Failure::into_error`].
#[derive(Debug)]
pub struct Failure {
    error: anyhow::Error,
    hint: Option<Hint>,
}

impl Failure {
    /// The lockfile could not be read.
    pub fn lockfile(error: LockfileError) -> Self {
        Self {
            hint: Hint::for_lockfile(&error),
            error: error.into(),
        }
    }

    /// The environment metadata could not be read.
    pub fn environment(error: EnvMetadataError) -> Self {
        Self {
            hint: Hint::for_environment(&error),
            error: error.into(),
        }
    }

    pub(crate) fn hint(&self) -> Option<Hint> {
        self.hint
    }

    /// The error, once the hint is no longer wanted.
    pub fn into_error(self) -> anyhow::Error {
        self.error
    }
}

impl<E: Into<anyhow::Error>> From<E> for Failure {
    fn from(error: E) -> Self {
        Self {
            error: error.into(),
            hint: None,
        }
    }
}
