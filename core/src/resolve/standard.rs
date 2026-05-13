// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{fmt, result::Result, sync::Arc};

use camino::Utf8PathBuf;
use reqwest_middleware::ClientWithMiddleware;

use crate::{
    auth::HTTPAuthentication,
    env::{
        discovery::DiscoveryError, index::IndexEnvironmentAsync,
        local_directory::LocalDirectoryEnvironment,
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

pub type LocalEnvResolver = EnvResolver<LocalDirectoryEnvironment>;

pub type RemoteIndexResolver<Policy> =
    SequentialResolver<EnvResolver<IndexEnvironmentAsync<Policy>>>;

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

pub fn standard_local_resolver(local_env: LocalDirectoryEnvironment) -> LocalEnvResolver {
    EnvResolver { env: local_env }
}

pub fn standard_index_resolver<Policy: HTTPAuthentication>(
    client: ClientWithMiddleware,
    urls: Vec<url::Url>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<AsSyncResolveTokio<RemoteIndexResolver<Policy>>, DiscoveryError> {
    // Each user-configured URL is a discovery root. Do not fetch
    // `sysand-index-config.json` here: resolver construction happens for
    // commands and bindings before we know whether an index dependency is
    // needed at all. The env resolves discovery lazily on first index use.
    let envs: Vec<EnvResolver<IndexEnvironmentAsync<Policy>>> = urls
        .into_iter()
        .map(|discovery_root| {
            let env = IndexEnvironmentAsync::from_discovery_root(
                client.clone(),
                auth_policy.clone(),
                discovery_root,
            );
            EnvResolver { env }
        })
        .collect();
    Ok(SequentialResolver::new(envs).to_tokio_sync(runtime))
}

// TODO: Replace most of these arguments by some general CLIOptions object
pub fn standard_resolver<Policy: HTTPAuthentication>(
    cwd: Option<Utf8PathBuf>,
    local_env: Option<LocalDirectoryEnvironment>,
    client: Option<ClientWithMiddleware>,
    index_urls: Option<Vec<url::Url>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<StandardResolver<Policy>, DiscoveryError> {
    let file_resolver = standard_file_resolver(cwd);
    let local_resolver = local_env.map(standard_local_resolver);
    let remote_resolver = client
        .clone()
        .map(|x| standard_remote_resolver(x, runtime.clone(), auth_policy.clone()));
    let index_resolver = client
        .zip(index_urls)
        .map(|(client, urls)| standard_index_resolver(client, urls, runtime, auth_policy))
        .transpose()?;

    Ok(StandardResolver(CombinedResolver {
        file_resolver: Some(file_resolver),
        local_resolver,
        remote_resolver,
        index_resolver,
    }))
}
