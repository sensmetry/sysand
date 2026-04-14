// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::resolve::{ResolutionOutcome, ResolveRead, ResolveReadAsync};
use futures::StreamExt as _;
use std::iter::Flatten;

/// Takes a sequence of similar resolvers, and tries them in sequence.
/// First resolves all versions in the first environment, then all
/// in the second, ...
#[derive(Debug)]
pub struct SequentialResolver<R> {
    inner: Vec<R>,
}

impl<R> SequentialResolver<R> {
    pub fn new<I: IntoIterator<Item = R>>(resolvers: I) -> Self {
        SequentialResolver {
            inner: resolvers.into_iter().collect(),
        }
    }
}

impl<R: ResolveRead> ResolveRead for SequentialResolver<R> {
    type Error = R::Error;

    type ProjectStorage = R::ProjectStorage;

    type ResolvedStorages = Flatten<std::vec::IntoIter<<R as ResolveRead>::ResolvedStorages>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let mut iters = vec![];
        let mut any_supported = false;
        let mut msgs = vec![];

        for resolver in &self.inner {
            match resolver.resolve_read(uri)? {
                ResolutionOutcome::Resolved(storages) => {
                    any_supported = true;
                    iters.push(storages)
                }
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    msgs.push(msg);
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    any_supported = true;
                    msgs.push(msg);
                }
            }
        }

        if !iters.is_empty() {
            Ok(ResolutionOutcome::Resolved(iters.into_iter().flatten()))
        } else if any_supported {
            Ok(ResolutionOutcome::Unresolvable(format!(
                "unresolvable: {:?}",
                msgs
            )))
        } else {
            Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "unsupported IRI: {:?}",
                msgs
            )))
        }
    }
}

impl<R: ResolveReadAsync> ResolveReadAsync for SequentialResolver<R> {
    type Error = R::Error;

    type ProjectStorage = R::ProjectStorage;

    type ResolvedStorages = futures::stream::Flatten<
        futures::stream::Iter<std::vec::IntoIter<<R as ResolveReadAsync>::ResolvedStorages>>,
    >;

    async fn resolve_read_async(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let outcomes = futures::future::join_all(
            self.inner
                .iter()
                .map(|resolver| resolver.resolve_read_async(uri)),
        )
        .await;

        let mut streams = vec![];
        let mut any_supported = false;
        let mut msgs = vec![];

        for outcome in outcomes {
            match outcome? {
                ResolutionOutcome::Resolved(storages) => {
                    any_supported = true;
                    streams.push(storages)
                }
                ResolutionOutcome::UnsupportedIRIType(msg) => {
                    msgs.push(msg);
                }
                ResolutionOutcome::Unresolvable(msg) => {
                    any_supported = true;
                    msgs.push(msg);
                }
            }
        }

        if !streams.is_empty() {
            Ok(ResolutionOutcome::Resolved(
                futures::stream::iter(streams).flatten(),
            ))
        } else if any_supported {
            Ok(ResolutionOutcome::Unresolvable(format!(
                "unresolvable: {:?}",
                msgs
            )))
        } else {
            Ok(ResolutionOutcome::UnsupportedIRIType(format!(
                "unsupported IRI: {:?}",
                msgs
            )))
        }
    }
}

#[cfg(test)]
#[path = "./sequential_tests.rs"]
mod tests;
