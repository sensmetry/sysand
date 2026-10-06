// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

// Resolve IRIs in an environment
use crate::{
    env::{ReadEnvironment, ReadEnvironmentAsync},
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead, ResolveReadAsync},
};

/// Resolves a usage to the versions of its project an environment holds,
/// highest first (see [`highest_first`])
#[derive(Debug)]
pub struct EnvResolver<Env> {
    pub env: Env,
}

/// `versions`, highest first by semver precedence, so that the solver, which
/// takes the first candidate a source lists among those it may choose,
/// chooses the highest. An environment need not list its versions in order
/// (a local one lists them as installed); an index already does, and keeps
/// its order. The sort is stable: versions of equal precedence (differing in
/// build metadata only) keep their order. Versions that are not semver, and
/// those that could not be read, come last, in their order.
// TODO: avoid reparsing: the solver parses each version again. Make
// `ReadEnvironment::versions` return `semver::Version`, so that each
// environment is responsible for its versions being valid
fn highest_first<E>(
    versions: impl IntoIterator<Item = Result<String, E>>,
) -> Vec<Result<String, E>> {
    let mut versions: Vec<_> = versions
        .into_iter()
        .map(|version| {
            let parsed = version
                .as_ref()
                .ok()
                .and_then(|v| semver::Version::parse(v).ok());
            (parsed, version)
        })
        .collect();
    versions.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) => b.cmp_precedence(a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    versions.into_iter().map(|(_, version)| version).collect()
}

impl<Env: ReadEnvironment> ResolveRead for EnvResolver<Env> {
    type Error = Env::ReadError;

    type ProjectStorage = Env::InterchangeProjectRead;

    type ResolvedStorages = Vec<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let id = resolve.id().into_string();
        let versions = highest_first(self.env.versions(&id)?);

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

    type ResolvedStorages =
        futures::stream::Iter<
            <Vec<
                Result<
                    <Self as ResolveReadAsync>::ProjectStorage,
                    <Self as ResolveReadAsync>::Error,
                >,
            > as IntoIterator>::IntoIter,
        >;

    async fn resolve_read_async(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        use futures::StreamExt as _;

        let id = resolve.id().into_string();
        let versions: Vec<Result<String, _>> = self.env.versions_async(&id).await?.collect().await;
        let versions = highest_first(versions);
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

#[cfg(test)]
#[path = "./env_tests.rs"]
mod tests;
