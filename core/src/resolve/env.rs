// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

// Resolve IRIs in an environment

use crate::{
    env::{ReadEnvironment, ReadEnvironmentAsync},
    resolve::{ResolutionOutcome, ResolveRead, ResolveReadAsync},
};

#[derive(Debug)]
pub struct EnvResolver<Env> {
    pub env: Env,
}

impl<Env: ReadEnvironment> ResolveRead for EnvResolver<Env> {
    type Error = Env::ReadError;

    type ProjectStorage = Env::InterchangeProjectRead;

    type ResolvedStorages = Vec<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let versions = self.env.versions(uri)?;

        let projects = versions.into_iter().map(
            |version| -> Result<Env::InterchangeProjectRead, Env::ReadError> {
                self.env.get_project(uri.clone(), version?)
            },
        );

        Ok(ResolutionOutcome::Resolved(projects.collect()))
    }
}

impl<Env: ReadEnvironmentAsync> ResolveReadAsync for EnvResolver<Env> {
    type Error = Env::ReadError;

    type ProjectStorage = Env::InterchangeProjectRead;

    type ResolvedStorages = futures::stream::Iter<
        <Vec<
            Result<
                <EnvResolver<Env> as ResolveReadAsync>::ProjectStorage,
                <EnvResolver<Env> as ResolveReadAsync>::Error,
            >,
        > as IntoIterator>::IntoIter,
    >;

    async fn resolve_read_async(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        use futures::StreamExt as _;

        let versions: Vec<Result<String, _>> = self.env.versions_async(uri).await?.collect().await;

        let projects = futures::future::join_all(
            versions
                .into_iter()
                .map(|version| async { self.env.get_project_async(uri.clone(), version?).await }),
        )
        .await;

        Ok(ResolutionOutcome::Resolved(futures::stream::iter(projects)))
    }
}
