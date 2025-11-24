// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt, path::PathBuf, result::Result, sync::Arc};

use reqwest_middleware::ClientWithMiddleware;
use typed_path::Utf8UnixPath;

use crate::{
    env::{
        local_directory::LocalDirectoryEnvironment, memory::MemoryStorageEnvironment,
        reqwest_http::HTTPEnvironmentAsync,
    },
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead, local_kpar::LocalKParProject, local_src::LocalSrcProject,
        reference::ProjectReference,
    },
    resolve::{
        AsSyncResolveTokio, ResolveRead, ResolveReadAsync,
        combined::CombinedResolver,
        env::EnvResolver,
        file::FileResolver,
        gix_git::GitResolver,
        remote::{RemotePriority, RemoteResolver},
        reqwest_http::HTTPResolverAsync,
        sequential::SequentialResolver,
    },
};

#[derive(Debug, ProjectRead)]
pub enum AnyProject {
    LocalSrc(LocalSrcProject),
    LocalKpar(LocalKParProject),
    // RemoteSrc(ReqwestSrcProjectAsync),
    // RemoteKpar(ReqwestKparDownloadedProject),
}

pub type OverrideProject = ProjectReference<AnyProject>;

pub type OverrideEnvironment = MemoryStorageEnvironment<OverrideProject>;

pub type OverrideResolver = EnvResolver<OverrideEnvironment>;

// pub type OverrideResolver = NullResolver;

pub type LocalEnvResolver = EnvResolver<LocalDirectoryEnvironment>;

pub type RemoteIndexResolver = SequentialResolver<EnvResolver<HTTPEnvironmentAsync>>;

type StandardResolverInner = CombinedResolver<
    FileResolver,
    LocalEnvResolver,
    OverrideResolver,
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
    overrides: Vec<(String, String, OverrideProject)>,
    client: Option<ClientWithMiddleware>,
    index_urls: Option<Vec<url::Url>>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> StandardResolver {
    let file_resolver = standard_file_resolver(cwd);
    let remote_resolver = client
        .clone()
        .map(|x| standard_remote_resolver(x, runtime.clone()));
    let local_resolver = local_env_path.map(standard_local_resolver);
    let index_resolver = client
        .zip(index_urls)
        .map(|(client, urls)| standard_index_resolver(client, urls, runtime.clone()));

    StandardResolver(CombinedResolver {
        file_resolver: Some(file_resolver),
        local_resolver,
        // override_resolver: NO_RESOLVER,
        override_resolver: Some(EnvResolver {
            env: MemoryStorageEnvironment::from(overrides),
        }),
        remote_resolver,
        index_resolver,
    })
}
