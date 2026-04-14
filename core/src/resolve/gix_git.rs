// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::component::Scheme;
use thiserror::Error;

use crate::{
    project::gix_git_download::{GixDownloadedError, GixDownloadedProject},
    resolve::{
        ResolutionOutcome, ResolveRead,
        file::SCHEME_FILE,
        reqwest_http::{SCHEME_HTTP, SCHEME_HTTPS},
    },
};

#[derive(Debug)]
pub struct GitResolver {}

#[derive(Error, Debug)]
pub enum GitResolverError {
    #[error(transparent)]
    GitProject(#[from] GixDownloadedError),
}

pub const SCHEME_SSH: &Scheme = Scheme::new_or_panic("ssh");
pub const SCHEME_GIT_SSH: &Scheme = Scheme::new_or_panic("git+ssh");
pub const SCHEME_GIT_FILE: &Scheme = Scheme::new_or_panic("git+file");
pub const SCHEME_GIT_HTTP: &Scheme = Scheme::new_or_panic("git+http");
pub const SCHEME_GIT_HTTPS: &Scheme = Scheme::new_or_panic("git+https");

impl ResolveRead for GitResolver {
    type Error = GitResolverError;

    type ProjectStorage = GixDownloadedProject;

    type ResolvedStorages = std::iter::Once<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let scheme = uri.scheme();

        if ![
            SCHEME_HTTP,
            SCHEME_HTTPS,
            SCHEME_FILE,
            SCHEME_SSH,
            SCHEME_GIT_HTTP,
            SCHEME_GIT_HTTPS,
            SCHEME_GIT_FILE,
            SCHEME_GIT_SSH,
        ]
        .contains(&scheme)
        {
            return Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "url scheme `{}` of IRI `{}` is not known to be git-compatible",
                scheme,
                uri.as_str()
            )));
        }

        Ok(ResolutionOutcome::Resolved(std::iter::once(
            // TODO: use trim_prefix() once it's stable
            GixDownloadedProject::new(uri.as_str().strip_prefix("git+").unwrap_or(uri.as_str()))
                .map_err(|e| e.into()),
        )))
    }

    fn resolve_read_raw<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        if let Some(stripped_uri) = uri.as_ref().strip_prefix("git+") {
            self.default_resolve_read_raw(stripped_uri)
        } else {
            self.default_resolve_read_raw(uri)
        }
    }
}

#[cfg(test)]
#[path = "./gix_git_tests.rs"]
mod tests;
