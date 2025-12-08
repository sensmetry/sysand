// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt, result::Result, sync::Arc};

use camino::{Utf8Path, Utf8PathBuf};
use reqwest_middleware::ClientWithMiddleware;
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
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
        utils::FsIoError,
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
pub enum AnyProject<Policy: HTTPAuthentication> {
    LocalSrc(LocalSrcProject),
    LocalKpar(LocalKParProject),
    RemoteSrc(AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>),
    RemoteKpar(AsSyncProjectTokio<ReqwestKparDownloadedProject<Policy>>),
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
impl<Policy: HTTPAuthentication> AnyProject<Policy> {
    pub fn try_from_source<P: AsRef<Utf8Path>>(
        source: Source,
        project_root: P,
        auth_policy: Arc<Policy>,
        client: ClientWithMiddleware,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Result<Self, TryFromSourceError> {
        match source {
            Source::LocalKpar { kpar_path } => Ok(AnyProject::LocalKpar(
                LocalKParProject::new_guess_root_nominal(
                    project_root.as_ref().join(kpar_path.as_str()),
                    kpar_path.as_str(),
                )
                .map_err(TryFromSourceError::LocalKpar)?,
            )),
            Source::LocalSrc { src_path } => {
                let nominal_path = src_path.as_str().into();
                let project_path = project_root.as_ref().join(&nominal_path);
                Ok(AnyProject::LocalSrc(LocalSrcProject {
                    nominal_path: Some(nominal_path),
                    project_path,
                }))
            }
            Source::RemoteKpar {
                remote_kpar,
                remote_kpar_size: _,
            } => Ok(AnyProject::RemoteKpar(
                ReqwestKparDownloadedProject::<Policy>::new_guess_root(
                    remote_kpar,
                    client,
                    auth_policy,
                )
                .map_err(TryFromSourceError::RemoteKpar)?
                .to_tokio_sync(runtime),
            )),
            Source::RemoteSrc { remote_src } => Ok(AnyProject::RemoteSrc(
                ReqwestSrcProjectAsync::<Policy> {
                    client,
                    url: reqwest::Url::parse(&remote_src).map_err(TryFromSourceError::RemoteSrc)?,
                    auth_policy,
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

pub type OverrideProject<Policy> = ProjectReference<AnyProject<Policy>>;

pub type OverrideEnvironment<Policy> = MemoryStorageEnvironment<OverrideProject<Policy>>;

pub type OverrideResolver<Policy> = MemoryResolver<AcceptAll, OverrideProject<Policy>>;

pub type LocalEnvResolver = EnvResolver<LocalDirectoryEnvironment>;

pub type RemoteIndexResolver<Policy> =
    SequentialResolver<EnvResolver<HTTPEnvironmentAsync<Policy>>>;

type StandardResolverInner<Policy> = CombinedResolver<
    FileResolver,
    LocalEnvResolver,
    RemoteResolver<AsSyncResolveTokio<HTTPResolverAsync<Policy>>, GitResolver>,
    AsSyncResolveTokio<RemoteIndexResolver<Policy>>,
>;

pub struct StandardResolver<Policy: HTTPAuthentication>(StandardResolverInner<Policy>);

impl<Policy: HTTPAuthentication> fmt::Debug for StandardResolver<Policy> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CliResolver").field(&self.0).finish()
    }
}

impl<Policy: HTTPAuthentication> ResolveRead for StandardResolver<Policy> {
    type Error = <StandardResolverInner<Policy> as ResolveRead>::Error;

    type ProjectStorage = <StandardResolverInner<Policy> as ResolveRead>::ProjectStorage;

    type ResolvedStorages = <StandardResolverInner<Policy> as ResolveRead>::ResolvedStorages;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<crate::resolve::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.0.resolve_read(uri)
    }
}

pub fn standard_file_resolver(cwd: Option<Utf8PathBuf>) -> FileResolver {
    FileResolver {
        sandbox_roots: None,
        relative_path_root: cwd,
    }
}

pub fn standard_remote_resolver<Policy: HTTPAuthentication>(
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> RemoteResolver<AsSyncResolveTokio<HTTPResolverAsync<Policy>>, GitResolver> {
    RemoteResolver {
        http_resolver: Some(
            HTTPResolverAsync {
                client,
                lax: true,
                auth_policy, //prefer_ranged: true,
            }
            .to_tokio_sync(runtime),
        ),
        git_resolver: Some(GitResolver {}),
        priority: RemotePriority::PreferHTTP,
    }
}

pub fn standard_local_resolver(local_env_path: Utf8PathBuf) -> LocalEnvResolver {
    EnvResolver {
        env: LocalDirectoryEnvironment {
            environment_path: local_env_path,
        },
    }
}

pub fn standard_index_resolver<Policy: HTTPAuthentication>(
    client: ClientWithMiddleware,
    urls: Vec<url::Url>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> AsSyncResolveTokio<RemoteIndexResolver<Policy>> {
    SequentialResolver::new(urls.into_iter().map(|url| EnvResolver {
        env: HTTPEnvironmentAsync {
            client: client.clone(),
            base_url: url.clone(),
            prefer_src: true,
            auth_policy: auth_policy.clone(),
            //try_ranged: true,
        },
    }))
    .to_tokio_sync(runtime)
}

// TODO: Replace most of these arguments by some general CLIOptions object
pub fn standard_resolver<Policy: HTTPAuthentication>(
    cwd: Option<Utf8PathBuf>,
    local_env_path: Option<Utf8PathBuf>,
    client: Option<ClientWithMiddleware>,
    index_urls: Option<Vec<url::Url>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> StandardResolver<Policy> {
    let file_resolver = standard_file_resolver(cwd);
    let local_resolver = local_env_path.map(standard_local_resolver);
    let remote_resolver = client
        .clone()
        .map(|x| standard_remote_resolver(x, runtime.clone(), auth_policy.clone()));
    let index_resolver = client
        .zip(index_urls)
        .map(|(client, urls)| standard_index_resolver(client, urls, runtime, auth_policy));

    StandardResolver(CombinedResolver {
        file_resolver: Some(file_resolver),
        local_resolver,
        remote_resolver,
        index_resolver,
    })
}
