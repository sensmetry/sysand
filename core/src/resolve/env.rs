// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

// Resolve IRIs in an environment
use crate::{
    env::{ReadEnvironment, ReadEnvironmentAsync},
    project::utils::Identifier,
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead, ResolveReadAsync},
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
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let id = Identifier::from_interchange_usage(resolve.usage()).to_string();
        let versions = self.env.versions(&id)?;

        let projects: Self::ResolvedStorages = versions
            .into_iter()
            .map(
                |version| -> Result<Env::InterchangeProjectRead, Env::ReadError> {
                    self.env.get_project(&id, version?)
                },
            )
            .collect();
        if projects.is_empty() {
            Ok(ResolutionOutcome::NotFound {
                reason: String::from("environment does not contain this project"),
            })
        } else {
            Ok(ResolutionOutcome::Resolved(projects))
        }
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
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        use futures::StreamExt as _;

        let id = Identifier::from_interchange_usage(resolve.usage()).to_string();
        let versions: Vec<Result<String, _>> = self.env.versions_async(&id).await?.collect().await;
        if versions.is_empty() {
            return Ok(ResolutionOutcome::NotFound {
                reason: String::from("environment does not contain this project"),
            });
        }

        let projects = futures::future::join_all(
            versions
                .into_iter()
                .map(|version| async { self.env.get_project_async(&id, version?).await }),
        )
        .await;

        Ok(ResolutionOutcome::Resolved(futures::stream::iter(projects)))
    }
}
