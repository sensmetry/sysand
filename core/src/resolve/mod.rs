// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt::Debug, sync::Arc};

use crate::{
    env::{SyncStreamIter, utils::ErrorBound},
    model::{
        InterchangeProjectUsage, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::{AsAsyncProject, AsSyncProjectTokio, ProjectRead, ProjectReadAsync},
};

use camino::Utf8Path;
use futures::stream::StreamExt as _;

pub mod combined;
pub mod env;
#[cfg(feature = "filesystem")]
pub mod file;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod gix_git;
pub mod memory;
#[cfg(feature = "networking")]
pub mod net_utils;
pub mod null;
pub mod priority;
pub mod remote;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod reqwest_http;
pub mod sequential;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod standard;
pub mod typed_resolver;

#[derive(Debug)]
pub enum ResolutionOutcome<T> {
    /// Successfully resolved a `T`. If `T` is a collection/iterator,
    /// it must contain at least one element
    Resolved(T),
    /// Resolution failed due to an unsupported type of usage
    UnsupportedUsageType {
        usage: InterchangeProjectUsage,
        reason: String,
    },
    /// The supplied usage was invalid. Must not be used when InterchangeProjectUsage
    /// (non-raw) is supplied
    InvalidUsage(
        InterchangeProjectUsageRaw,
        InterchangeProjectValidationError,
    ),
    /// Usage was not found
    NotFound(InterchangeProjectUsage, String),
    /// Resolution failed due to an invalid usage that is in principle supported
    Unresolvable(String),
}

impl<T> ResolutionOutcome<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, op: F) -> ResolutionOutcome<U> {
        match self {
            Self::Resolved(t) => ResolutionOutcome::Resolved(op(t)),
            Self::UnsupportedUsageType { usage, reason } => {
                ResolutionOutcome::UnsupportedUsageType { usage, reason }
            }
            Self::InvalidUsage(usage, err) => ResolutionOutcome::InvalidUsage(usage, err),
            Self::NotFound(usage, reason) => ResolutionOutcome::NotFound(usage, reason),
            Self::Unresolvable(msg) => ResolutionOutcome::Unresolvable(msg),
        }
    }
}

/// This is only ussed to resolve "resource" IRIs, new style usages
/// go directly through their specific resolvers.
pub trait ResolveRead {
    type Error: ErrorBound;

    type ProjectStorage: ProjectRead; // + Clone;
    type ResolvedStorages: IntoIterator<Item = Result<Self::ProjectStorage, Self::Error>>;

    // TODO: move path-specific docs to FileResolver
    /// `base_path` is absolute/relative to CWD path of the project to which this usage
    /// belongs. Relative path usages will be resolved using `base_path` as base.
    /// If `base_path` is `None` and usage is a relative path, resolution will fail
    fn default_resolve_read_raw(
        &self,
        usage: &InterchangeProjectUsageRaw,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        match usage.validate() {
            Ok(u) => self.resolve_read(&u, base_path),
            Err(err) => Ok(ResolutionOutcome::InvalidUsage(usage.to_owned(), err)),
        }
    }

    fn resolve_read_raw(
        &self,
        usage: &InterchangeProjectUsageRaw,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.default_resolve_read_raw(usage, base_path)
    }

    fn resolve_read(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error>;

    /// Treat this `ResolveRead` as a (trivial) `ResolveReadAsync`
    fn to_async(self) -> AsAsyncResolve<Self>
    where
        Self: Sized,
    {
        AsAsyncResolve { inner: self }
    }
}

pub trait ResolveReadAsync {
    type Error: ErrorBound;

    type ProjectStorage: ProjectReadAsync;
    type ResolvedStorages: futures::Stream<Item = Result<Self::ProjectStorage, Self::Error>>;

    fn default_resolve_read_raw_async(
        &self,
        usage: &InterchangeProjectUsageRaw,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> impl Future<Output = Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error>> {
        async move {
            match usage.validate() {
                Ok(u) => self.resolve_read_async(&u, base_path).await,
                Err(err) => Ok(ResolutionOutcome::InvalidUsage(usage.to_owned(), err)),
            }
        }
    }

    fn resolve_read_raw_async(
        &self,
        usage: &InterchangeProjectUsageRaw,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> impl Future<Output = Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error>> {
        async move { self.default_resolve_read_raw_async(usage, base_path).await }
    }

    fn resolve_read_async(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> impl Future<Output = Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error>>;

    // Maybe make this return an associated type instead? Would, for example, allow
    // .as_async.as_tokio_sync == .as_tokio_sync.as_async == id
    /// Treat this `ResolveReadAsync` as a `ResolveRead` using the provided tokio runtime.
    fn to_tokio_sync(self, runtime: Arc<tokio::runtime::Runtime>) -> AsSyncResolveTokio<Self>
    where
        Self: Sized,
    {
        AsSyncResolveTokio {
            runtime,
            inner: self,
        }
    }
}

/// Wrapper intended to warp an `ResolveRead`, indicating that it should
/// be treated as a (trivial) `ResolveReadAsync`.
#[repr(transparent)]
#[derive(Debug)]
pub struct AsAsyncResolve<T> {
    inner: T,
}

impl<T: ResolveRead> ResolveReadAsync for AsAsyncResolve<T>
where
    for<'a> <<T as ResolveRead>::ProjectStorage as ProjectRead>::SourceReader<'a>: Unpin,
{
    type Error = <T as ResolveRead>::Error;

    type ProjectStorage = AsAsyncProject<<T as ResolveRead>::ProjectStorage>;

    type ResolvedStorages = futures::stream::Iter<
        std::iter::Map<
            <<T as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter,
            fn(
                Result<<T as ResolveRead>::ProjectStorage, <T as ResolveRead>::Error>,
            ) -> Result<
                AsAsyncProject<<T as ResolveRead>::ProjectStorage>,
                <T as ResolveRead>::Error,
            >,
        >,
    >;

    async fn resolve_read_async(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(self.inner.resolve_read(usage, base_path)?.map(|storages| {
            let projects_map: std::iter::Map<_, fn(_) -> _> = storages
                .into_iter()
                .map(|proj| Ok(AsAsyncProject { inner: proj? }));

            futures::stream::iter(projects_map)
        }))
    }
}

/// Wrapper intended to wrap `ResolveReadAsync`, indicating that it should be treated as a
/// `ResolveRead`, using a provided tokio runtime.
#[derive(Debug)]
pub struct AsSyncResolveTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
}

impl<T: ResolveReadAsync> ResolveRead for AsSyncResolveTokio<T>
where
    <T as ResolveReadAsync>::ResolvedStorages: Unpin,
{
    type Error = <T as ResolveReadAsync>::Error;

    type ProjectStorage = AsSyncProjectTokio<<T as ResolveReadAsync>::ProjectStorage>;

    type ResolvedStorages = SyncStreamIter<
        futures::stream::Map<
            <T as ResolveReadAsync>::ResolvedStorages,
            // TODO: Replace this with a more concrete type
            Box<
                dyn Fn(
                    Result<<T as ResolveReadAsync>::ProjectStorage, <T as ResolveReadAsync>::Error>,
                ) -> Result<
                    AsSyncProjectTokio<<T as ResolveReadAsync>::ProjectStorage>,
                    <T as ResolveReadAsync>::Error,
                >,
            >,
        >,
    >;

    fn resolve_read(
        &self,
        usage: &InterchangeProjectUsage,
        base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(self
            .runtime
            .block_on(self.inner.resolve_read_async(usage, base_path))?
            .map(|storages| {
                let runtime_clone = self.runtime.clone();

                let inner: futures::stream::Map<_, Box<dyn Fn(_) -> _>> =
                    storages.map(Box::new(move |project| {
                        Ok(AsSyncProjectTokio {
                            runtime: runtime_clone.clone(),
                            inner: project?,
                        })
                    }));

                SyncStreamIter {
                    runtime: self.runtime.clone(),
                    inner,
                }
            }))
    }
}
