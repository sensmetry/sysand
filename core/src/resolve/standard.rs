// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fmt,
    path::{Path, PathBuf},
    result::Result,
    sync::Arc,
};

use reqwest_middleware::ClientWithMiddleware;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    env::{
        local_directory::LocalDirectoryEnvironment, memory::MemoryStorageEnvironment,
        reqwest_http::HTTPEnvironmentAsync,
    },
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        AsSyncProjectTokio, ProjectRead, ProjectReadAsync,
        local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
        reference::ProjectReference,
        reqwest_kpar_download::{ReqwestKparDownloadedError, ReqwestKparDownloadedProject},
        reqwest_src::ReqwestSrcProjectAsync,
        utils::{FsIoError, ToPathBuf},
    },
    resolve::{
        AsSyncResolveTokio, ResolveRead, ResolveReadAsync,
        combined::CombinedResolver,
        env::EnvResolver,
        file::FileResolver,
        gix_git::GitResolver,
        memory::{AcceptAll, MemoryResolver},
        remote::{RemotePriority, RemoteResolver},
        reqwest_http::HTTPResolverAsync,
        sequential::SequentialResolver,
    },
};

#[derive(Debug, ProjectRead)]
pub enum AnyProject {
    LocalSrc(LocalSrcProject),
    LocalKpar(LocalKParProject),
    RemoteSrc(AsSyncProjectTokio<ReqwestSrcProjectAsync>),
    RemoteKpar(AsSyncProjectTokio<ReqwestKparDownloadedProject>),
}

#[derive(Error, Debug)]
pub enum TryFromSourceError {
    #[error("unsupported source\n{0}")]
    UnsupportedSource(String),
    #[error(transparent)]
    LocalKpar(Box<FsIoError>),
    #[error(transparent)]
    RemoteKpar(ReqwestKparDownloadedError),
    #[error(transparent)]
    RemoteSrc(url::ParseError),
}

// TODO: Find a better solution going from source to project.
// Preferably one that can also be used when syncing.
impl AnyProject {
    pub fn try_from_source<P: AsRef<Path>>(
        source: Source,
        project_root: P,
        client: ClientWithMiddleware,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Result<Self, TryFromSourceError> {
        match source {
            Source::LocalKpar { kpar_path } => Ok(AnyProject::LocalKpar(
                LocalKParProject::new_guess_root_nominal(
                    project_root.as_ref().join(&kpar_path),
                    kpar_path,
                )
                .map_err(TryFromSourceError::LocalKpar)?,
            )),
            Source::LocalSrc { src_path } => Ok(AnyProject::LocalSrc(LocalSrcProject {
                nominal_path: Some(src_path.to_path_buf()),
                project_path: project_root.as_ref().join(src_path),
            })),
            Source::RemoteKpar {
                remote_kpar,
                remote_kpar_size: _,
            } => Ok(AnyProject::RemoteKpar(
                ReqwestKparDownloadedProject::new_guess_root(remote_kpar, client)
                    .map_err(TryFromSourceError::RemoteKpar)?
                    .to_tokio_sync(runtime),
            )),
            Source::RemoteSrc { remote_src } => Ok(AnyProject::RemoteSrc(
                ReqwestSrcProjectAsync {
                    client,
                    url: reqwest::Url::parse(&remote_src).map_err(TryFromSourceError::RemoteSrc)?,
                }
                .to_tokio_sync(runtime),
            )),
            _ => Err(TryFromSourceError::UnsupportedSource(format!(
                "{:?}",
                source
            ))),
        }
    }
}

pub type OverrideProject = ProjectReference<AnyProject>;

pub type OverrideEnvironment = MemoryStorageEnvironment<OverrideProject>;

pub type OverrideResolver = MemoryResolver<AcceptAll, OverrideProject>;

pub type LocalEnvResolver = EnvResolver<LocalDirectoryEnvironment>;

pub type RemoteIndexResolver = SequentialResolver<EnvResolver<HTTPEnvironmentAsync>>;

type StandardResolverInner = CombinedResolver<
    FileResolver,
    LocalEnvResolver,
    RemoteResolver<AsSyncResolveTokio<HTTPResolverAsync>, GitResolver>,
    AsSyncResolveTokio<RemoteIndexResolver>,
>;

pub struct StandardResolver(StandardResolverInner);

impl fmt::Debug for StandardResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CliResolver").field(&self.0).finish()
    }
}

impl ResolveRead for StandardResolver {
    type Error = <StandardResolverInner as ResolveRead>::Error;

    type ProjectStorage = <StandardResolverInner as ResolveRead>::ProjectStorage;

    type ResolvedStorages = <StandardResolverInner as ResolveRead>::ResolvedStorages;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<crate::resolve::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.0.resolve_read(uri)
    }
}

pub fn standard_file_resolver(cwd: Option<PathBuf>) -> FileResolver {
    FileResolver {
        sandbox_roots: None,
        relative_path_root: cwd,
    }
}

pub fn standard_remote_resolver(
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> RemoteResolver<AsSyncResolveTokio<HTTPResolverAsync>, GitResolver> {
    RemoteResolver {
        http_resolver: Some(
            HTTPResolverAsync {
                client,
                lax: true,
                //prefer_ranged: true,
            }
            .to_tokio_sync(runtime),
        ),
        git_resolver: Some(GitResolver {}),
        priority: RemotePriority::PreferHTTP,
    }
}

pub fn standard_local_resolver(local_env_path: PathBuf) -> LocalEnvResolver {
    EnvResolver {
        env: LocalDirectoryEnvironment {
            environment_path: local_env_path,
        },
    }
}

pub fn standard_index_resolver(
    client: ClientWithMiddleware,
    urls: Vec<url::Url>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> AsSyncResolveTokio<RemoteIndexResolver> {
    SequentialResolver::new(urls.into_iter().map(|url| EnvResolver {
        env: HTTPEnvironmentAsync {
            client: client.clone(),
            base_url: url.clone(),
            prefer_src: true,
            //try_ranged: true,
        },
    }))
    .to_tokio_sync(runtime)
}

// TODO: Replace most of these arguments by some general CLIOptions object
pub fn standard_resolver(
    cwd: Option<PathBuf>,
    local_env_path: Option<PathBuf>,
    client: Option<ClientWithMiddleware>,
    index_urls: Option<Vec<url::Url>>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> StandardResolver {
    let file_resolver = standard_file_resolver(cwd);
    let local_resolver = local_env_path.map(standard_local_resolver);
    let remote_resolver = client
        .clone()
        .map(|x| standard_remote_resolver(x, runtime.clone()));
    let index_resolver = client
        .zip(index_urls)
        .map(|(client, urls)| standard_index_resolver(client, urls, runtime));

    StandardResolver(CombinedResolver {
        file_resolver: Some(file_resolver),
        local_resolver,
        remote_resolver,
        index_resolver,
    })
}
