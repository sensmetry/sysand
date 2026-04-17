// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{fmt::Debug, iter::Peekable};

use indexmap::IndexMap;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{CanonicalizationError, ProjectRead, cached::CachedProject},
    resolve::{ResolutionOutcome, ResolveRead, null::NullResolver},
};

/// Implements "standard" resolution logic given a set of individual resolvers.
/// Use sysand::resolve::null::NullResolver to skip any of the steps.
/// The logic is as follows:
/// 1. Do not resolve any further if file_resolver is successful, otherwise go to step 2.
/// 2. If remote_resolver produces any results, discard any that do not point to a valid
///    project (i.e. do not produce both a info and meta). If at least one project is found
///    proceed to 4. (skipping 3.)
/// 3. Take whatever results are produced by remote_resolver and proceed to step 4.
/// 4. If local_resolver resolves anything, collect all the results. Iterate over the results
///    from previous steps, but interleave results from local_resolver when they have
///    identical hashes. Any results from local_resolver that were not interleaved are returned
///    at the end.
///
///    Cached values are returned exactly once (so if the underlying resolver gives duplicates
///    they will appear cached only one time).
///
/// The above procedure basically amounts to:
/// - file_resolver represents private projects
/// - remote_resolver is prioritised, but may be ignored if it does not resolve valid projects
///   (typically due to using non-resolving URLs to reference a resource)
/// - registry_resolver is what will typically be hit when using non-resolvable IRIs
/// - local_resolver serves to provide a cache, but may contain "dangling" cached projects
///
/// Each resolver is optional, and can be skipped by passing `None`. `NO_RESOLVER` is a typed `None`
/// value that can be used to avoid ambiguous typing.
#[derive(Debug)]
pub struct CombinedResolver<FileResolver, LocalResolver, RemoteResolver, RegistryResolver> {
    /// A resolver for whatever is considered a local file in the environment,
    /// would *typically* accept only file:// URLs
    pub file_resolver: Option<FileResolver>,
    /// A resolver for whatever is considered local environments/local caches
    pub local_resolver: Option<LocalResolver>,
    /// A resolver for whatever is considered remote URLs, would typically resolves
    /// http(s) and git-URLs, as well as, possibly, FTP, rsync, scp, ...
    pub remote_resolver: Option<RemoteResolver>,
    /// A resolver for whatever is considered a central project registry, typically
    /// resolves only urn:kpar:... names and, possibly, unresolvable http(s) URLs.
    pub index_resolver: Option<RegistryResolver>,
}

/// Utility resolver
pub const NO_RESOLVER: Option<NullResolver> = None;

#[derive(Error, Debug)]
pub enum CombinedResolverError<FileError, LocalError, RemoteError, RegistryError> {
    #[error(transparent)]
    File(FileError),
    #[error(transparent)]
    Local(LocalError),
    #[error(transparent)]
    Remote(RemoteError),
    #[error(transparent)]
    Registry(RegistryError),
}

/// Outcome of a standard resolution remembers the (resolver) source of the project.
/// Can either be taken apart or used directly as a project storage.
#[derive(Debug, ProjectRead)]
pub enum CombinedProjectStorage<
    FileProjectStorage: ProjectRead,
    LocalProjectStorage: ProjectRead,
    RemoteProjectStorage: ProjectRead,
    RegistryProjectStorage: ProjectRead,
> {
    FileProject(FileProjectStorage),
    RemoteProject(RemoteProjectStorage),
    RegistryProject(RegistryProjectStorage),
    CachedRemoteProject(CachedProject<LocalProjectStorage, RemoteProjectStorage>),
    CachedRegistryProject(CachedProject<LocalProjectStorage, RegistryProjectStorage>),
    DanglingLocalProject(LocalProjectStorage),
}

pub enum CombinedIteratorState<
    FileResolver: ResolveRead,
    RemoteResolver: ResolveRead,
    RegistryResolver: ResolveRead,
> {
    /// The IRI was resolved as a local path
    ResolvedFile(<<FileResolver as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter),
    /// The IRI was resolved to (at least one) valid remote project
    ResolvedRemote(
        Peekable<<<RemoteResolver as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter>,
    ),
    /// We rely on the remote registry
    ResolvedRegistry(
        <<RegistryResolver as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter,
    ),
    /// At most some local hits (not resolved otherwise) remain
    Done,
}

pub struct CombinedIterator<
    FileResolver: ResolveRead,
    LocalResolver: ResolveRead,
    RemoteResolver: ResolveRead,
    RegistryResolver: ResolveRead,
> {
    pub state: CombinedIteratorState<FileResolver, RemoteResolver, RegistryResolver>,
    pub locals: IndexMap<String, LocalResolver::ProjectStorage>,
}

impl<
    FileResolver: ResolveRead,
    LocalResolver: ResolveRead,
    RemoteResolver: ResolveRead,
    RegistryResolver: ResolveRead,
> Iterator for CombinedIterator<FileResolver, LocalResolver, RemoteResolver, RegistryResolver>
{
    type Item = Result<
        CombinedProjectStorage<
            FileResolver::ProjectStorage,
            LocalResolver::ProjectStorage,
            RemoteResolver::ProjectStorage,
            RegistryResolver::ProjectStorage,
        >,
        CombinedResolverError<
            FileResolver::Error,
            LocalResolver::Error,
            RemoteResolver::Error,
            RegistryResolver::Error,
        >,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.state {
            CombinedIteratorState::ResolvedFile(iter) => iter.next().map(|r| {
                r.map(CombinedProjectStorage::FileProject)
                    .map_err(CombinedResolverError::File)
            }),
            CombinedIteratorState::Done => self
                .locals
                .pop()
                .map(|v| Ok(CombinedProjectStorage::DanglingLocalProject(v.1))),
            CombinedIteratorState::ResolvedRemote(iter) => match iter.next() {
                Some(r) => Some(r.map_err(CombinedResolverError::Remote).map(|project| {
                    let cached = project
                        .checksum_canonical_hex()
                        .ok()
                        .flatten()
                        .and_then(|checksum| self.locals.shift_remove(&checksum));

                    if let Some(local_project) = cached {
                        CombinedProjectStorage::CachedRemoteProject(CachedProject::new(
                            local_project,
                            project,
                        ))
                    } else {
                        CombinedProjectStorage::RemoteProject(project)
                    }
                })),
                None => {
                    self.state = CombinedIteratorState::Done;
                    self.next()
                }
            },
            CombinedIteratorState::ResolvedRegistry(iter) => match iter.next() {
                Some(r) => Some(r.map_err(CombinedResolverError::Registry).map(|project| {
                    let cached = project
                        .checksum_canonical_hex()
                        .ok()
                        .flatten()
                        .and_then(|checksum| self.locals.shift_remove(&checksum));

                    if let Some(local_project) = cached {
                        CombinedProjectStorage::CachedRegistryProject(CachedProject::new(
                            local_project,
                            project,
                        ))
                    } else {
                        CombinedProjectStorage::RegistryProject(project)
                    }
                })),
                None => {
                    self.state = CombinedIteratorState::Done;
                    self.next()
                }
            },
        }
    }
}

impl<
    FileResolver: ResolveRead,
    LocalResolver: ResolveRead,
    RemoteResolver: ResolveRead,
    RegistryResolver: ResolveRead,
> ResolveRead for CombinedResolver<FileResolver, LocalResolver, RemoteResolver, RegistryResolver>
{
    type Error = CombinedResolverError<
        FileResolver::Error,
        LocalResolver::Error,
        RemoteResolver::Error,
        RegistryResolver::Error,
    >;

    type ProjectStorage = CombinedProjectStorage<
        FileResolver::ProjectStorage,
        LocalResolver::ProjectStorage,
        RemoteResolver::ProjectStorage,
        RegistryResolver::ProjectStorage,
    >;

    // TODO: Replace this with something more efficient
    type ResolvedStorages =
        CombinedIterator<FileResolver, LocalResolver, RemoteResolver, RegistryResolver>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let mut at_least_one_supports = false;

        // If the file resolver does not outright reject the IRI type,
        // use it.
        // TODO: autodetect git (and possibly other VCSs), and use appropriate (e.g. git) resolver for them.
        if let Some(file_resolver) = &self.file_resolver {
            match file_resolver
                .resolve_read(uri)
                .map_err(CombinedResolverError::File)?
            {
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("file resolver rejected IRI `{uri}`: {msg}");
                } // Just continue
                ResolutionOutcome::Resolved(r) => {
                    //at_least_one_supports = true;
                    return Ok(ResolutionOutcome::Resolved(CombinedIterator {
                        state: CombinedIteratorState::ResolvedFile(r.into_iter()),
                        locals: IndexMap::new(),
                    }));
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    return Ok(ResolutionOutcome::Unresolvable(format!(
                        "failed to resolve as file: {msg}"
                    )));
                }
            }
        }

        // Collect local cached projects
        let mut locals: IndexMap<String, LocalResolver::ProjectStorage> = IndexMap::new();

        if let Some(local_resolver) = &self.local_resolver {
            match local_resolver
                .resolve_read(uri)
                .map_err(CombinedResolverError::Local)?
            {
                ResolutionOutcome::Resolved(projects) => {
                    at_least_one_supports = true;
                    for res in projects {
                        match res {
                            Err(err) => {
                                log::debug!(
                                    "local resolver rejected project with IRI `{uri}`: {err}",
                                );
                            }
                            Ok(project) => match project.checksum_canonical_hex() {
                                Ok(Some(checksum)) => {
                                    locals.insert(checksum, project);
                                }
                                Ok(None) => {
                                    log::debug!(
                                        "local resolver rejected project with IRI `{uri}` due to missing canonical checksum",
                                    );
                                }
                                Err(err) => {
                                    log::debug!(
                                        "local resolver rejected project with IRI `{uri}`: {err}",
                                    );
                                }
                            },
                        }
                    }
                }
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("local resolver rejected IRI `{uri}`: {msg}");
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!("local resolver unable to resolve IRI `{uri}`: {msg}");
                }
            };
        }

        // Need in reverse order for pop-ing
        locals.reverse();

        if let Some(remote_resolver) = &self.remote_resolver {
            // Skip over remote resolution if unresolvable or if only invalid projects are produced.
            match remote_resolver
                .resolve_read(uri)
                .map_err(CombinedResolverError::Remote)?
            {
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("remote resolver rejected IRI `{uri}`: {msg}");
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!("remote resolver unable to resolve IRI `{uri}`: {msg}");
                }
                ResolutionOutcome::Resolved(remote_projects) => {
                    at_least_one_supports = true;
                    // See if at least one project is valid
                    let mut remote_projects = remote_projects.into_iter().peekable();

                    loop {
                        match remote_projects.peek() {
                            Some(Err(err)) => {
                                log::debug!(
                                    "remote resolver skipping project for IRI `{uri}` due to: {err}"
                                );
                                remote_projects.next();
                            }
                            Some(Ok(project)) => {
                                if project.is_definitely_invalid() {
                                    remote_projects.next();
                                    continue;
                                }

                                match project.get_project() {
                                    Ok((Some(_), Some(_))) => {
                                        // Found at least one nominally valid project
                                        return Ok(ResolutionOutcome::Resolved(CombinedIterator {
                                            state: CombinedIteratorState::ResolvedRemote(
                                                remote_projects,
                                            ),
                                            locals,
                                        }));
                                    }
                                    Ok(_) => {
                                        log::debug!(
                                            "remote resolver skipping project for IRI `{uri}` due to missing info/meta"
                                        );
                                        remote_projects.next();
                                    }
                                    Err(err) => {
                                        log::debug!(
                                            "remote resolver skipping project for IRI `{uri}`: {err}"
                                        );
                                        remote_projects.next();
                                    }
                                }
                            }
                            None => {
                                log::debug!(
                                    "remote resolver unable to find valid project for IRI `{uri}`"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Finally try the centralised registry if neither file/remote gave anything useful
        if let Some(index_resolver) = &self.index_resolver {
            match index_resolver
                .resolve_read(uri)
                .map_err(CombinedResolverError::Registry)?
            {
                ResolutionOutcome::Resolved(x) => {
                    return Ok(ResolutionOutcome::Resolved(CombinedIterator {
                        state: CombinedIteratorState::ResolvedRegistry(x.into_iter()),
                        locals,
                    }));
                }
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("registry resolver rejected IRI `{uri}` due to: {msg}");
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!("registry resolver unable to resolve IRI `{uri}`: {msg}");
                }
            };
        }

        // As a last resort, use only locally cached projects, if any were found
        if !at_least_one_supports {
            Ok(ResolutionOutcome::UnsupportedIRIType(
                "no resolver accepted the IRI".to_owned(),
            ))
        } else if locals.is_empty() {
            Ok(ResolutionOutcome::Unresolvable(
                "no resolver was able to resolve the IRI".to_owned(),
            ))
        } else {
            Ok(ResolutionOutcome::Resolved(CombinedIterator {
                state: CombinedIteratorState::Done,
                locals,
            }))
        }
    }
}

#[cfg(test)]
#[path = "./combined_tests.rs"]
mod tests;
