// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    model::InterchangeProjectUsage,
    project::gix_git_download::{GixDownloadedError, GixDownloadedProject},
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::scheme::{
        SCHEME_FILE, SCHEME_GIT_FILE, SCHEME_GIT_HTTP, SCHEME_GIT_HTTPS, SCHEME_GIT_SSH,
        SCHEME_HTTP, SCHEME_HTTPS, SCHEME_SSH,
    },
};

#[derive(Debug)]
pub struct GitResolver {}

#[derive(Error, Debug)]
pub enum GitResolverError {
    #[error(transparent)]
    GitProject(#[from] GixDownloadedError),
}

impl ResolveRead for GitResolver {
    type Error = GitResolverError;

    type ProjectStorage = GixDownloadedProject;

    type ResolvedStorages = std::iter::Once<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        match resolve.usage() {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            } => {
                let scheme = resource.scheme();

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
                    return Ok(ResolutionOutcome::UnsupportedUsageType {
                        reason: format!(
                            "url scheme `{scheme}` of IRI `{resource}` is not known to be git-compatible"
                        ),
                    });
                }

                Ok(ResolutionOutcome::Resolved(std::iter::once(
                    // TODO: use trim_prefix() once it's stable
                    GixDownloadedProject::new(
                        resource
                            .as_str()
                            .strip_prefix("git+")
                            .unwrap_or(resource.as_str()),
                    )
                    .map_err(|e| e.into()),
                )))
            }
            InterchangeProjectUsage::Directory { .. } => {
                Ok(ResolutionOutcome::UnsupportedUsageType {
                    reason: String::from("not a git usage"),
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "./gix_git_tests.rs"]
mod tests;
