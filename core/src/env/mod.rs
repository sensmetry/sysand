// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use sha2::Digest;

use thiserror::Error;

use crate::project::{ProjectMut, ProjectRead};

// pub mod utils;

// Implementations
#[cfg(feature = "filesystem")]
pub mod local_directory;
pub mod memory;
pub mod null;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod reqwest_http;

pub mod utils;

pub fn segment_uri_generic<S: AsRef<str>, D: Digest>(uri: S) -> std::vec::IntoIter<String>
where
    digest::Output<D>: core::fmt::LowerHex,
{
    let mut hasher = D::new();
    hasher.update(uri.as_ref());

    vec![format!("{:x}", hasher.finalize())].into_iter()
}

pub trait ReadEnvironment {
    type ReadError: std::error::Error + std::fmt::Debug;

    type UriIter: IntoIterator<Item = Result<String, Self::ReadError>>;
    fn uris(&self) -> Result<Self::UriIter, Self::ReadError>;

    type VersionIter: IntoIterator<Item = Result<String, Self::ReadError>>;
    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError>;

    type InterchangeProjectRead: ProjectRead + std::fmt::Debug;
    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError>;

    // Utilities

    fn has<S: AsRef<str>>(&self, uri: S) -> Result<bool, Self::ReadError> {
        Ok(self
            .uris()?
            .into_iter()
            .filter_map(Result::ok)
            .any(|u: String| u == uri.as_ref()))
    }

    fn has_version<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<bool, Self::ReadError> {
        Ok(self
            .versions(&uri)?
            .into_iter()
            .filter_map(Result::ok)
            .any(|v: String| v == version.as_ref()))
    }

    fn candidate_projects<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Vec<Self::InterchangeProjectRead>, Self::ReadError> {
        let versions: Result<Vec<_>, _> = self.versions(&uri)?.into_iter().collect();

        let projects: Result<Vec<_>, _> = versions?
            .into_iter()
            .map(|v| self.get_project(&uri, v))
            .collect();

        projects
    }
}

#[derive(Error, Debug)]
pub enum PutProjectError<WE, CE> {
    #[error("{0}")]
    WriteError(WE),
    #[error("{0}")]
    CallbackError(CE),
}

pub trait WriteEnvironment {
    type WriteError: std::error::Error + std::fmt::Debug;

    type InterchangeProjectMut: ProjectMut;

    // TODO: Should this be replaced by a transactional interface?
    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        // Callback allows the implementation to gracefully recover
        // in case of an error, to just "allocate"
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>;

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError>;

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError>;
}
