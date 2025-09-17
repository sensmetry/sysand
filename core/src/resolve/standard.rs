// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use crate::{
    env::{local_directory::LocalDirectoryEnvironment, reqwest_http::HTTPEnvironment},
    resolve::{
        ResolveRead,
        combined::CombinedResolver,
        env::EnvResolver,
        file::FileResolver,
        gix_git::GitResolver,
        remote::{RemotePriority, RemoteResolver},
        reqwest_http::HTTPResolver,
    },
};
use reqwest::blocking::Client;

pub type LocalEnvResolver = EnvResolver<LocalDirectoryEnvironment>;

pub type RemoteIndexResolver = EnvResolver<HTTPEnvironment>;

type StandardResolverInner = CombinedResolver<
    FileResolver,
    LocalEnvResolver,
    RemoteResolver<HTTPResolver, GitResolver>,
    RemoteIndexResolver,
>;

pub struct StandardResolver(StandardResolverInner);

impl std::fmt::Debug for StandardResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    ) -> std::result::Result<crate::resolve::ResolutionOutcome<Self::ResolvedStorages>, Self::Error>
    {
        self.0.resolve_read(uri)
    }
}

pub fn standard_file_resolver(cwd: Option<PathBuf>) -> FileResolver {
    FileResolver {
        sandbox_roots: None,
        relative_path_root: cwd,
    }
}

pub fn standard_remote_resolver(client: Client) -> RemoteResolver<HTTPResolver, GitResolver> {
    RemoteResolver {
        http_resolver: Some(HTTPResolver {
            client,
            lax: true,
            prefer_ranged: true,
        }),
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

pub fn standard_index_resolver(client: Client, base_url: url::Url) -> RemoteIndexResolver {
    EnvResolver {
        env: HTTPEnvironment {
            client,
            base_url,
            prefer_src: true,
            try_ranged: true,
        },
    }
}

// TODO: Replace most of these arguments by some general CLIOptions object
pub fn standard_resolver(
    cwd: Option<PathBuf>,
    local_env_path: Option<PathBuf>,
    client: Option<Client>,
    index_base_url: Option<url::Url>,
) -> StandardResolver {
    let file_resolver = standard_file_resolver(cwd);
    let remote_resolver = client.clone().map(standard_remote_resolver);
    let local_resolver = local_env_path.map(standard_local_resolver);
    let index_resolver = client
        .zip(index_base_url)
        .map(|(client, base_url)| standard_index_resolver(client, base_url));

    StandardResolver(CombinedResolver {
        file_resolver: Some(file_resolver),
        local_resolver,
        remote_resolver,
        index_resolver,
    })
}
