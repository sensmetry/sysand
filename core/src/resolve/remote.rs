// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::io::{self, Read};

use thiserror::Error;

use crate::{
    context::ProjectContext,
    lock::Source,
    project::{CanonicalizationError, ProjectRead},
    resolve::{ResolveRead, null::NullResolver},
};

use super::ResolutionOutcome;

#[derive(Debug, Clone, Copy)]
pub enum RemotePriority {
    PreferGit,
    PreferHTTP,
}

#[derive(Debug)]
pub struct RemoteResolver<HTTPResolver, GitResolver> {
    pub http_resolver: Option<HTTPResolver>,
    pub git_resolver: Option<GitResolver>,
    pub priority: RemotePriority,
}

/// Utility resolver
pub const NO_RESOLVER: Option<NullResolver> = None;

#[derive(Error, Debug)]
pub enum RemoteResolverError<HTTPError, GitError> {
    #[error(transparent)]
    HTTPResolver(HTTPError),
    #[error(transparent)]
    GitResolver(GitError),
}

#[derive(Debug)]
pub enum RemoteProject<HTTPProject, GitProject> {
    HTTPProject(HTTPProject),
    GitProject(GitProject),
}

#[derive(Error, Debug)]
pub enum RemoteProjectError<HTTPError, GitError> {
    #[error(transparent)]
    HTTPRead(HTTPError),
    #[error(transparent)]
    GitRead(GitError),
}

#[derive(Debug)]
pub enum RemoteSourceReader<HTTPReader, GitReader> {
    HTTPReader(HTTPReader),
    GitReader(GitReader),
}

impl<HTTPReader: Read, GitReader: Read> Read for RemoteSourceReader<HTTPReader, GitReader> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            RemoteSourceReader::HTTPReader(reader) => reader.read(buf),
            RemoteSourceReader::GitReader(reader) => reader.read(buf),
        }
    }
}

impl<HTTPProject: ProjectRead, GitProject: ProjectRead> ProjectRead
    for RemoteProject<HTTPProject, GitProject>
{
    type Error = RemoteProjectError<HTTPProject::Error, GitProject::Error>;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.get_project().map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.get_project().map_err(RemoteProjectError::GitRead)
            }
        }
    }

    type SourceReader<'a>
        = RemoteSourceReader<HTTPProject::SourceReader<'a>, GitProject::SourceReader<'a>>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => project
                .read_source(path)
                .map(RemoteSourceReader::HTTPReader)
                .map_err(RemoteProjectError::HTTPRead),
            RemoteProject::GitProject(project) => project
                .read_source(path)
                .map(RemoteSourceReader::GitReader)
                .map_err(RemoteProjectError::GitRead),
        }
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.sources(ctx).map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.sources(ctx).map_err(RemoteProjectError::GitRead)
            }
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            RemoteProject::HTTPProject(project) => project.is_definitely_invalid(),
            RemoteProject::GitProject(project) => project.is_definitely_invalid(),
        }
    }

    fn get_info(&self) -> Result<Option<crate::model::InterchangeProjectInfoRaw>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.get_info().map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.get_info().map_err(RemoteProjectError::GitRead)
            }
        }
    }

    fn get_meta(&self) -> Result<Option<crate::model::InterchangeProjectMetadataRaw>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.get_meta().map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.get_meta().map_err(RemoteProjectError::GitRead)
            }
        }
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.version().map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.version().map_err(RemoteProjectError::GitRead)
            }
        }
    }

    fn usage(&self) -> Result<Option<Vec<crate::model::InterchangeProjectUsageRaw>>, Self::Error> {
        match self {
            RemoteProject::HTTPProject(project) => {
                project.usage().map_err(RemoteProjectError::HTTPRead)
            }
            RemoteProject::GitProject(project) => {
                project.usage().map_err(RemoteProjectError::GitRead)
            }
        }
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        match self {
            RemoteProject::HTTPProject(project) => project
                .checksum_canonical_hex()
                .map_err(|e| e.map_project_read(RemoteProjectError::HTTPRead)),
            RemoteProject::GitProject(project) => project
                .checksum_canonical_hex()
                .map_err(|e| e.map_project_read(RemoteProjectError::GitRead)),
        }
    }
}

pub struct ResolvedRemote<HTTPResolver: ResolveRead, GitResolver: ResolveRead> {
    pub resolved_http: Option<<HTTPResolver::ResolvedStorages as IntoIterator>::IntoIter>,
    pub resolved_git: Option<<GitResolver::ResolvedStorages as IntoIterator>::IntoIter>,
    pub priority: RemotePriority,
}

impl<HTTPResolver: ResolveRead, GitResolver: ResolveRead> Iterator
    for ResolvedRemote<HTTPResolver, GitResolver>
{
    type Item = Result<
        RemoteProject<HTTPResolver::ProjectStorage, GitResolver::ProjectStorage>,
        RemoteResolverError<HTTPResolver::Error, GitResolver::Error>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match self.priority {
            RemotePriority::PreferGit => {
                if let Some(primary_resolver) = &mut self.resolved_git {
                    if let Some(next_primary) = primary_resolver.next() {
                        let next = next_primary
                            .map(RemoteProject::GitProject)
                            .map_err(RemoteResolverError::GitResolver);

                        return Some(next);
                    } else {
                        self.resolved_git = None;
                    }
                }

                if let Some(secondary_resolver) = &mut self.resolved_http {
                    if let Some(next_secondary) = secondary_resolver.next() {
                        let next = next_secondary
                            .map(RemoteProject::HTTPProject)
                            .map_err(RemoteResolverError::HTTPResolver);

                        return Some(next);
                    } else {
                        self.resolved_http = None;
                    }
                }

                None
            }
            RemotePriority::PreferHTTP => {
                if let Some(primary_resolver) = &mut self.resolved_http {
                    if let Some(next_primary) = primary_resolver.next() {
                        let next = next_primary
                            .map(RemoteProject::HTTPProject)
                            .map_err(RemoteResolverError::HTTPResolver);

                        return Some(next);
                    } else {
                        self.resolved_http = None;
                    }
                }

                if let Some(secondary_resolver) = &mut self.resolved_git {
                    if let Some(next_secondary) = secondary_resolver.next() {
                        let next = next_secondary
                            .map(RemoteProject::GitProject)
                            .map_err(RemoteResolverError::GitResolver);

                        return Some(next);
                    } else {
                        self.resolved_git = None;
                    }
                }

                None
            }
        }
    }
}

impl<HTTPResolver: ResolveRead, GitResolver: ResolveRead> ResolveRead
    for RemoteResolver<HTTPResolver, GitResolver>
{
    type Error = RemoteResolverError<HTTPResolver::Error, GitResolver::Error>;

    type ProjectStorage = RemoteProject<HTTPResolver::ProjectStorage, GitResolver::ProjectStorage>;

    type ResolvedStorages = ResolvedRemote<HTTPResolver, GitResolver>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let resolved_http = if let Some(http_resolver) = &self.http_resolver {
            match http_resolver
                .resolve_read(uri)
                .map_err(RemoteResolverError::HTTPResolver)?
            {
                ResolutionOutcome::Resolved(resolved) => Some(resolved.into_iter()),
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("HTTP resolver rejected IRI: {msg}");
                    None
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    log::debug!("HTTP resolver failed to resolve IRI: {msg}");
                    None
                }
            }
        } else {
            None
        };

        let resolved_git = if let Some(git_resolver) = &self.git_resolver {
            match git_resolver
                .resolve_read(uri)
                .map_err(RemoteResolverError::GitResolver)?
            {
                ResolutionOutcome::Resolved(resolved) => Some(resolved.into_iter()),
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("git resolver rejected IRI: {msg}");
                    None
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    log::debug!("git resolver failed to resolve IRI: {msg}");
                    None
                }
            }
        } else {
            None
        };

        Ok(crate::resolve::ResolutionOutcome::Resolved(
            ResolvedRemote {
                resolved_http,
                resolved_git,
                priority: self.priority,
            },
        ))
    }
}
