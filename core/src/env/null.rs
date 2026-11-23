// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    fmt::Debug,
    iter::{Empty, empty},
    marker::PhantomData,
};

use thiserror::Error;

use crate::{env::ReadEnvironment, project::ProjectRead};

#[derive(Debug)]
pub struct NullEnvironment<Pr> {
    phantom: PhantomData<Pr>,
}

impl<Pr> Default for NullEnvironment<Pr> {
    fn default() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyEnvironmentError {
    #[error("null environment contains no packages")]
    NullEnvironmentIsEmpty,
}

impl<Pr> NullEnvironment<Pr> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<Pr: ProjectRead + Debug> ReadEnvironment for NullEnvironment<Pr> {
    type ReadError = EmptyEnvironmentError;

    type UriIter = Empty<Result<String, EmptyEnvironmentError>>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        Ok(empty())
    }

    type VersionIter = Empty<Result<String, EmptyEnvironmentError>>;

    fn versions<S: AsRef<str>>(&self, _uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        Ok(empty())
    }

    type InterchangeProjectRead = Pr;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        _uri: S,
        _version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Err(EmptyEnvironmentError::NullEnvironmentIsEmpty)
    }
}
