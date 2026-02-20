// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    io::{self, Read},
    iter::Peekable,
};

use indexmap::IndexMap;
use thiserror::Error;

use crate::{
    model::{ProjectHash, project_hash_raw},
    project::ProjectRead,
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

#[derive(Error, Debug)]
pub enum CombinedReadError<FileError, LocalError, RemoteError, RegistryError> {
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
#[derive(Debug)]
pub enum CombinedProjectStorage<
    FileProjectStorage,
    LocalProjectStorage,
    RemoteProjectStorage,
    RegistryProjectStorage,
> {
    FileProject(FileProjectStorage),
    RemoteProject(RemoteProjectStorage),
    RegistryProject(RegistryProjectStorage),
    CachedRemoteProject(LocalProjectStorage, RemoteProjectStorage),
    CachedRegistryProject(LocalProjectStorage, RegistryProjectStorage),
    DanglingLocalProject(LocalProjectStorage),
}

pub enum CombinedSourceReader<FileReader, LocalReader, RemoteReader, RegistryReader> {
    FileProject(FileReader),
    LocalProject(LocalReader),
    RemoteProject(RemoteReader),
    RegistryProject(RegistryReader),
}

impl<FileReader: Read, LocalReader: Read, RemoteReader: Read, RegistryReader: Read> Read
    for CombinedSourceReader<FileReader, LocalReader, RemoteReader, RegistryReader>
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            CombinedSourceReader::FileProject(reader) => reader.read(buf),
            CombinedSourceReader::LocalProject(reader) => reader.read(buf),
            CombinedSourceReader::RemoteProject(reader) => reader.read(buf),
            CombinedSourceReader::RegistryProject(reader) => reader.read(buf),
        }
    }
}

impl<
    FileProjectStorage: ProjectRead,
    LocalProjectStorage: ProjectRead,
    RemoteProjectStorage: ProjectRead,
    RegistryProjectStorage: ProjectRead,
> ProjectRead
    for CombinedProjectStorage<
        FileProjectStorage,
        LocalProjectStorage,
        RemoteProjectStorage,
        RegistryProjectStorage,
    >
{
    type Error = CombinedReadError<
        FileProjectStorage::Error,
        LocalProjectStorage::Error,
        RemoteProjectStorage::Error,
        RegistryProjectStorage::Error,
    >;

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
            CombinedProjectStorage::FileProject(project) => {
                project.get_project().map_err(CombinedReadError::File)
            }
            CombinedProjectStorage::RemoteProject(project) => {
                project.get_project().map_err(CombinedReadError::Remote)
            }
            CombinedProjectStorage::RegistryProject(project) => {
                project.get_project().map_err(CombinedReadError::Registry)
            }
            CombinedProjectStorage::CachedRemoteProject(project, _) => {
                project.get_project().map_err(CombinedReadError::Local)
            }
            CombinedProjectStorage::CachedRegistryProject(project, _) => {
                project.get_project().map_err(CombinedReadError::Local)
            }
            CombinedProjectStorage::DanglingLocalProject(project) => {
                project.get_project().map_err(CombinedReadError::Local)
            }
        }
    }

    type SourceReader<'a>
        = CombinedSourceReader<
        FileProjectStorage::SourceReader<'a>,
        LocalProjectStorage::SourceReader<'a>,
        RemoteProjectStorage::SourceReader<'a>,
        RegistryProjectStorage::SourceReader<'a>,
    >
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            CombinedProjectStorage::FileProject(project) => project
                .read_source(path)
                .map_err(CombinedReadError::File)
                .map(CombinedSourceReader::FileProject),
            CombinedProjectStorage::RemoteProject(project) => project
                .read_source(path)
                .map_err(CombinedReadError::Remote)
                .map(CombinedSourceReader::RemoteProject),
            CombinedProjectStorage::RegistryProject(project) => project
                .read_source(path)
                .map_err(CombinedReadError::Registry)
                .map(CombinedSourceReader::RegistryProject),
            CombinedProjectStorage::CachedRemoteProject(project, _) => project
                .read_source(path)
                .map_err(CombinedReadError::Local)
                .map(CombinedSourceReader::LocalProject),
            CombinedProjectStorage::CachedRegistryProject(project, _) => project
                .read_source(path)
                .map_err(CombinedReadError::Local)
                .map(CombinedSourceReader::LocalProject),
            CombinedProjectStorage::DanglingLocalProject(project) => project
                .read_source(path)
                .map_err(CombinedReadError::Local)
                .map(CombinedSourceReader::LocalProject),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            CombinedProjectStorage::FileProject(proj) => proj.is_definitely_invalid(),
            CombinedProjectStorage::RemoteProject(proj) => proj.is_definitely_invalid(),
            CombinedProjectStorage::RegistryProject(proj) => proj.is_definitely_invalid(),
            CombinedProjectStorage::CachedRemoteProject(proj, _) => proj.is_definitely_invalid(),
            CombinedProjectStorage::CachedRegistryProject(proj, _) => proj.is_definitely_invalid(),
            CombinedProjectStorage::DanglingLocalProject(proj) => proj.is_definitely_invalid(),
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            CombinedProjectStorage::FileProject(proj) => proj.sources(),
            CombinedProjectStorage::RemoteProject(proj) => proj.sources(),
            CombinedProjectStorage::RegistryProject(proj) => proj.sources(),
            CombinedProjectStorage::CachedRemoteProject(_, proj) => proj.sources(),
            CombinedProjectStorage::CachedRegistryProject(_, proj) => proj.sources(),
            CombinedProjectStorage::DanglingLocalProject(proj) => proj.sources(),
        }
    }
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
    pub locals: IndexMap<ProjectHash, LocalResolver::ProjectStorage>,
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
                        .get_project()
                        .ok()
                        .and_then(|(spec, meta)| spec.zip(meta))
                        .and_then(|(spec, meta)| {
                            self.locals.shift_remove(&project_hash_raw(&spec, &meta))
                        });

                    if let Some(local_project) = cached {
                        CombinedProjectStorage::CachedRemoteProject(local_project, project)
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
                        .get_project()
                        .ok()
                        .and_then(|(spec, meta)| spec.zip(meta))
                        .and_then(|(spec, meta)| {
                            self.locals.shift_remove(&project_hash_raw(&spec, &meta))
                        });

                    if let Some(local_project) = cached {
                        CombinedProjectStorage::CachedRegistryProject(local_project, project)
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
            let mut rejected = vec![];
            match file_resolver
                .resolve_read(uri)
                .map_err(CombinedResolverError::File)?
            {
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("File resolver rejects IRI '{}' due to: {}", uri, msg);
                } // Just continue
                ResolutionOutcome::Resolved(r) => {
                    //at_least_one_supports = true;
                    return Ok(ResolutionOutcome::Resolved(CombinedIterator {
                        state: CombinedIteratorState::ResolvedFile(r.into_iter()),
                        locals: IndexMap::new(),
                    }));
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    rejected.push(msg);
                }
            }

            if !rejected.is_empty() {
                return Ok(ResolutionOutcome::Unresolvable(format!(
                    "failed to resolve as file: {:?}",
                    rejected
                )));
            }
        }

        // Collect local cached projects
        let mut locals: IndexMap<ProjectHash, LocalResolver::ProjectStorage> = IndexMap::new();

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
                                    "Local resolver rejected project with IRI {} due to: {:?}",
                                    uri,
                                    err
                                );
                            }
                            Ok(project) => match project.get_project() {
                                Ok((Some(info), Some(meta))) => {
                                    locals.insert(project_hash_raw(&info, &meta), project);
                                }
                                Ok(_) => {
                                    log::debug!(
                                        "Local resolver rejected project with IRI {} due to missing project/info",
                                        uri
                                    );
                                }
                                Err(err) => {
                                    log::debug!(
                                        "Local resolver rejected project with IRI {} due to: {:?}",
                                        uri,
                                        err
                                    );
                                }
                            },
                        }
                    }
                }
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    log::debug!("Local resolver rejected IRI {} due to: {}", uri, msg);
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!(
                        "Local resolver unable to resolve IRI {} due to: {}",
                        uri,
                        msg
                    );
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
                    log::debug!("Remote resolver rejects IRI {} due to: {}", uri, msg);
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!(
                        "Remote resolver unable to resolve IRI {} due to: {}",
                        uri,
                        msg
                    );
                }
                ResolutionOutcome::Resolved(remote_projects) => {
                    at_least_one_supports = true;
                    // See if at least one project is valid
                    let mut remote_projects = remote_projects.into_iter().peekable();

                    loop {
                        match remote_projects.peek() {
                            Some(Err(err)) => {
                                log::debug!(
                                    "Remote resolver skipping projrect for IRI {} due to: {}",
                                    uri,
                                    err
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
                                            "Remote resolver skipping projrect for IRI {} due to missing info/meta",
                                            uri
                                        );
                                        remote_projects.next();
                                    }
                                    Err(err) => {
                                        log::debug!(
                                            "Remote resolver skipping projrect for IRI {} due to: {:?}",
                                            uri,
                                            err
                                        );
                                        remote_projects.next();
                                    }
                                }
                            }
                            None => {
                                log::debug!(
                                    "Remote resolver unable to find valid project for IRI {}",
                                    uri
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
                    log::debug!("Registry resolver rejects IRI {} due to: {}", uri, msg);
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    at_least_one_supports = true;
                    log::debug!(
                        "Registry resolver unable to resolve IRI {} due to: {}",
                        uri,
                        msg
                    );
                }
            };
        }

        // As a last resort, use only locally cached projects, if any were found
        if !at_least_one_supports {
            Ok(ResolutionOutcome::UnsupportedIRIType(
                "No resolver accepted the IRI".to_string(),
            ))
        } else if locals.is_empty() {
            Ok(ResolutionOutcome::Unresolvable(
                "No resolver was able to resolve the IRI".to_string(),
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
mod tests {
    use std::collections::HashMap;

    use fluent_uri::Iri;
    use indexmap::IndexMap;

    use crate::{
        info::do_info,
        model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
        project::memory::InMemoryProject,
        resolve::{
            ResolveRead,
            combined::{CombinedResolver, NO_RESOLVER},
            memory::{AcceptAll, MemoryResolver},
        },
    };

    fn minimal_project<S: AsRef<str>, T: AsRef<str>>(name: S, version: T) -> InMemoryProject {
        InMemoryProject {
            info: Some(InterchangeProjectInfoRaw {
                name: name.as_ref().to_string(),
                publisher: None,
                description: None,
                version: version.as_ref().to_string(),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: vec![],
            }),
            meta: Some(InterchangeProjectMetadataRaw {
                index: IndexMap::new(),
                created: "1970-01-01T00:00:00.000000000Z".to_string(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: None,
            }),
            files: HashMap::new(),
            nominal_sources: vec![],
        }
    }

    // const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");

    fn empty_any_resolver() -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
        Some(MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: HashMap::new(),
        })
    }

    fn single_project_any_resolver<S: AsRef<str>>(
        uri: S,
        project: InMemoryProject,
    ) -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
        let uri = Iri::parse(uri.as_ref().to_string()).unwrap();

        let mut projects = HashMap::new();

        projects.insert(uri, vec![project]);

        Some(MemoryResolver {
            iri_predicate: AcceptAll {},
            projects,
        })
    }

    // fn single_project_file_resolver<S: AsRef<str>>(
    //     uri: S,
    //     project: ProjectMemoryStorage,
    // ) -> MemoryResolver<AcceptScheme<'static>, ProjectMemoryStorage> {
    //     let uri = fluent_uri::Iri::parse(uri.as_ref().to_string()).unwrap();

    //     if uri.scheme() != SCHEME_FILE {
    //         panic!("Invalid IRI for file resolver");
    //     }

    //     let mut projects = HashMap::new();

    //     projects.insert(uri, project);

    //     MemoryResolver {
    //         iri_predicate: AcceptScheme {
    //             scheme: SCHEME_FILE,
    //         },
    //         projects: projects,
    //     }
    // }

    #[test]
    fn prefer_file_resolver_when_successful() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");
        let project_b = minimal_project("b", "3.2.1");

        let resolver = CombinedResolver {
            file_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            remote_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].0.name, "a");
    }

    #[test]
    fn prefer_file_resolver_even_when_unresolved() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");

        let resolver = CombinedResolver {
            file_resolver: empty_any_resolver(),
            remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        };

        let xs = do_info(example_uri, &resolver);

        assert!(xs.is_err())
    }

    #[test]
    fn skip_file_resolver_if_unsupported_iri() {
        let example_uri = "http://example.com";

        //let project_a = minimal_project("a", "1.2.3");
        let project_b = minimal_project("b", "3.2.1");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].0.name, "b");
    }

    #[test]
    fn prefer_remote_over_registry_if_valid_cached() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");
        let project_b = minimal_project("b", "3.2.1");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].0.name, "a");
    }

    #[test]
    fn prefer_remote_over_registry_if_valid_uncached() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");
        let project_b = minimal_project("b", "3.2.1");
        let project_c = minimal_project("c", "3.2.1");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_c.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 2);
        assert_eq!(xs[0].0.name, "a");
        assert_eq!(xs[1].0.name, "b");
    }

    #[test]
    fn skip_remote_if_unsupported_uncached() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");
        let project_b = minimal_project("b", "3.2.1");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: NO_RESOLVER,
            local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 2);
        assert_eq!(xs[0].0.name, "a");
        assert_eq!(xs[1].0.name, "b");
    }

    #[test]
    fn skip_remote_if_unsupported_cached() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: NO_RESOLVER,
            local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].0.name, "a");
    }

    #[test]
    fn skip_remote_if_unresolved_cached() {
        let example_uri = "http://example.com";

        let project_a = minimal_project("a", "1.2.3");

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: empty_any_resolver(),
            local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
            index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        };

        let xs = do_info(example_uri, &resolver).unwrap();

        assert_eq!(xs.len(), 1);
        assert_eq!(xs[0].0.name, "a");
    }

    #[test]
    fn unsupported_iri_test() {
        let example_uri = "http://example.com";

        let resolver = CombinedResolver {
            file_resolver: NO_RESOLVER,
            remote_resolver: NO_RESOLVER,
            local_resolver: NO_RESOLVER,
            index_resolver: NO_RESOLVER,
        };

        let Ok(crate::resolve::ResolutionOutcome::UnsupportedIRIType(_)) =
            resolver.resolve_read_raw(example_uri)
        else {
            panic!()
        };
    }

    #[test]
    fn unresolved_iri_test() {
        let example_uri = "http://example.com";

        let resolver = CombinedResolver {
            file_resolver: empty_any_resolver(),
            remote_resolver: empty_any_resolver(),
            local_resolver: empty_any_resolver(),
            index_resolver: empty_any_resolver(),
        };

        let Ok(crate::resolve::ResolutionOutcome::Unresolvable(_)) =
            resolver.resolve_read_raw(example_uri)
        else {
            panic!()
        };
    }
}
