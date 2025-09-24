// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::project::{ProjectMut, ProjectRead};

pub mod combined;
pub mod env;
#[cfg(feature = "filesystem")]
pub mod file;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod gix_git;
pub mod memory;
pub mod null;
pub mod remote;
pub mod replace;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod reqwest_http;
pub mod sequential;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod standard;

#[derive(Debug)]
pub enum ResolutionOutcome<T> {
    /// Successfully resolved a T
    Resolved(T),
    /// Resolution failed due to an unsupported type of IRI
    UnsupportedIRIType(String),
    /// Resolution failed due to an invalid IRI that is in principle supported
    Unresolvable(String),
}

impl<T> ResolutionOutcome<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, op: F) -> ResolutionOutcome<U> {
        match self {
            Self::Resolved(t) => ResolutionOutcome::Resolved(op(t)),
            Self::UnsupportedIRIType(e) => ResolutionOutcome::UnsupportedIRIType(e),
            Self::Unresolvable(e) => ResolutionOutcome::Unresolvable(e),
        }
    }
}

pub trait ResolveRead {
    type Error: std::error::Error + std::fmt::Debug;

    type ProjectStorage: ProjectRead;
    type ResolvedStorages: IntoIterator<Item = Result<Self::ProjectStorage, Self::Error>>;

    fn default_resolve_read_raw<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        match fluent_uri::Iri::parse(uri.as_ref().to_string()) {
            Ok(uri) => self.resolve_read(&uri),
            Err(err) => Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "Unable to parse IRI {}: {}",
                uri.as_ref(),
                err
            ))),
        }
    }

    fn resolve_read_raw<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.default_resolve_read_raw(uri)
    }

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error>;
}

// TODO: Figure out if this is really useful
pub trait ResolveWrite {
    type Error: std::error::Error + std::fmt::Debug;

    type ProjectStorage: ProjectMut;

    fn with_resolve_write_raw<S: AsRef<str>, F>(
        &self,
        uri: S,
        write_project: F,
    ) -> Result<ResolutionOutcome<Self::ProjectStorage>, Self::Error>
    where
        F: FnOnce(&mut Self::ProjectStorage) -> Result<(), Self::Error>,
    {
        match fluent_uri::Iri::parse(uri.as_ref().to_string()) {
            Ok(uri) => self.with_resolve_write(&uri, write_project),
            Err(err) => Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "Unable to parse IRI {}: {}",
                uri.as_ref(),
                err
            ))),
        }
    }

    fn with_resolve_write<F>(
        &self,
        uri: &fluent_uri::Iri<String>,
        write_project: F,
    ) -> Result<ResolutionOutcome<Self::ProjectStorage>, Self::Error>
    where
        F: FnOnce(&mut Self::ProjectStorage) -> Result<(), Self::Error>;
}
