use std::io::Read;

use thiserror::Error;

use crate::{
    project::ProjectRead,
    resolve::{ResolveRead, null::NullResolver},
};

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
    #[error("{0}")]
    HTTPResolverError(HTTPError),
    #[error("{0}")]
    GitResolverError(GitError),
}

#[derive(Debug)]
pub enum RemoteProject<HTTPProject, GitProject> {
    HTTPProject(HTTPProject),
    GitProject(GitProject),
}

#[derive(Error, Debug)]
pub enum RemoteProjectError<HTTPError, GitError> {
    #[error("{0}")]
    HTTPReadError(HTTPError),
    #[error("{0}")]
    GitReadError(GitError),
}

#[derive(Debug)]
pub enum RemoteSourceReader<HTTPReader, GitReader> {
    HTTPReader(HTTPReader),
    GitReader(GitReader),
}

impl<HTTPReader: Read, GitReader: Read> Read for RemoteSourceReader<HTTPReader, GitReader> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
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
            RemoteProject::HTTPProject(project) => project
                .get_project()
                .map_err(RemoteProjectError::HTTPReadError),
            RemoteProject::GitProject(project) => project
                .get_project()
                .map_err(RemoteProjectError::GitReadError),
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
                .map_err(RemoteProjectError::HTTPReadError),
            RemoteProject::GitProject(project) => project
                .read_source(path)
                .map(RemoteSourceReader::GitReader)
                .map_err(RemoteProjectError::GitReadError),
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            RemoteProject::HTTPProject(project) => project.sources(),
            RemoteProject::GitProject(project) => project.sources(),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            RemoteProject::HTTPProject(project) => project.is_definitely_invalid(),
            RemoteProject::GitProject(project) => project.is_definitely_invalid(),
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
                            .map_err(RemoteResolverError::GitResolverError);

                        return Some(next);
                    } else {
                        self.resolved_git = None;
                    }
                }

                if let Some(secondary_resolver) = &mut self.resolved_http {
                    if let Some(next_secondary) = secondary_resolver.next() {
                        let next = next_secondary
                            .map(RemoteProject::HTTPProject)
                            .map_err(RemoteResolverError::HTTPResolverError);

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
                            .map_err(RemoteResolverError::HTTPResolverError);

                        return Some(next);
                    } else {
                        self.resolved_http = None;
                    }
                }

                if let Some(secondary_resolver) = &mut self.resolved_git {
                    if let Some(next_secondary) = secondary_resolver.next() {
                        let next = next_secondary
                            .map(RemoteProject::GitProject)
                            .map_err(RemoteResolverError::GitResolverError);

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
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let resolved_http = if let Some(http_resolver) = &self.http_resolver {
            if let super::ResolutionOutcome::Resolved(resolved) = http_resolver
                .resolve_read(uri)
                .map_err(RemoteResolverError::HTTPResolverError)?
            {
                Some(resolved.into_iter())
            } else {
                None
            }
        } else {
            None
        };

        let resolved_git = if let Some(git_resolver) = &self.git_resolver {
            if let super::ResolutionOutcome::Resolved(resolved) = git_resolver
                .resolve_read(uri)
                .map_err(RemoteResolverError::GitResolverError)?
            {
                Some(resolved.into_iter())
            } else {
                None
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
