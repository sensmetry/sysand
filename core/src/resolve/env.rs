// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::Utf8Path;

// Resolve IRIs in an environment
use crate::{
    env::{ReadEnvironment, ReadEnvironmentAsync},
    model::InterchangeProjectUsage,
    project::utils::Identifier,
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
        usage: &InterchangeProjectUsage,
        _base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let identifier = Identifier::from_interchange_usage(usage);
        let versions = self.env.versions(&identifier)?;

        let projects: Self::ResolvedStorages = versions
            .into_iter()
            .map(
                |version| -> Result<Env::InterchangeProjectRead, Env::ReadError> {
                    self.env.get_project(&identifier, version?)
                },
            )
            .collect();
        if projects.is_empty() {
            Ok(ResolutionOutcome::NotFound(
                usage.to_owned(),
                String::from("no versions of the project found in environment"),
            ))
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
        usage: &InterchangeProjectUsage,
        _base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        use futures::StreamExt as _;
        let identifier = Identifier::from_interchange_usage(usage);

        let versions: Vec<Result<String, _>> =
            self.env.versions_async(&identifier).await?.collect().await;
        if versions.is_empty() {
            return Ok(ResolutionOutcome::NotFound(
                usage.to_owned(),
                String::from("no versions of the project found in environment"),
            ));
        }

        let projects = futures::future::join_all(
            versions
                .into_iter()
                .map(|version| async { self.env.get_project_async(&identifier, version?).await }),
        )
        .await;

        Ok(ResolutionOutcome::Resolved(futures::stream::iter(projects)))
    }
}
