// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    fmt::{Debug, Display},
    mem::discriminant,
    sync::Arc,
};

use crate::{
    env::{SyncStreamIter, utils::ErrorBound},
    model::InterchangeProjectUsage,
    project::{
        AsAsyncProject, AsSyncProjectTokio, ProjectRead, ProjectReadAsync,
        utils::{Identifier, wrapfs},
    },
};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
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

#[derive(Debug)]
pub enum ResolutionOutcome<T> {
    /// Successfully resolved a `T`. If `T` is a collection/iterator,
    /// it must contain at least one element
    Resolved(T),
    /// Resolution failed due to an unsupported type of usage
    UnsupportedUsageType { reason: String },
    /// Usage is supported, but was not found
    NotFound { reason: String },
    /// Resolution failed due to an invalid usage that is in principle supported
    Unresolvable { reason: String },
}

impl<T> ResolutionOutcome<T> {
    pub fn map<U, F: FnOnce(T) -> U>(self, op: F) -> ResolutionOutcome<U> {
        match self {
            Self::Resolved(t) => ResolutionOutcome::Resolved(op(t)),
            Self::UnsupportedUsageType { reason } => {
                ResolutionOutcome::UnsupportedUsageType { reason }
            }
            Self::NotFound { reason } => ResolutionOutcome::NotFound { reason },
            Self::Unresolvable { reason } => ResolutionOutcome::Unresolvable { reason },
        }
    }
}

/// Information needed to resolve a usage
#[derive(Debug, Clone, Eq)]
pub struct ResolutionInfo {
    usage: InterchangeProjectUsage,
    /// Base path to resolve this usage against. Not relevant for
    /// usages that do not involve filesystem paths
    base_path: Option<Utf8PathBuf>,
}

// It is incorrect to use the derived `Hash` impl for resolution, since pubgrub
// seemingly identifies packages by their hash, so e.g. usages of the same
// package that have different version requirements will be treated as
// referring to two distinct packages, and they will all be included in the
// solution
// Note that `PartialEq` effectively must match the behaviour of this due
// to the way we implement dependency solving
impl std::hash::Hash for ResolutionInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Mention all fields here to remember to update whenever
        // the struct changes
        let Self {
            usage,
            base_path: _,
        } = self;
        discriminant(&self.usage).hash(state);
        match usage {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            } => {
                resource.hash(state);
            }
            InterchangeProjectUsage::Directory {
                dir: _,
                publisher,
                name,
            } => {
                // TODO: maybe include absolute path (base_path + dir)?
                publisher.hash(state);
                name.hash(state);
            }
        }
    }
}

// This is for use in pubgrub only; for other usees it likely
// does not match expectations
impl PartialEq for ResolutionInfo {
    fn eq(&self, other: &Self) -> bool {
        // Mention all fields here to remember to update whenever
        // the struct changes
        let Self {
            usage: self_usage,
            base_path: _,
        } = self;
        let Self {
            usage: other_usage,
            base_path: _,
        } = other;
        match (self_usage, other_usage) {
            (
                InterchangeProjectUsage::Resource {
                    resource: self_resource,
                    version_constraint: _,
                },
                InterchangeProjectUsage::Resource {
                    resource,
                    version_constraint: _,
                },
            ) => self_resource == resource,
            (
                InterchangeProjectUsage::Directory {
                    dir: _,
                    publisher: self_publisher,
                    name: self_name,
                },
                InterchangeProjectUsage::Directory {
                    dir: _,
                    publisher,
                    name,
                },
            ) => self_publisher == publisher && self_name == name,
            (
                InterchangeProjectUsage::Directory { .. },
                InterchangeProjectUsage::Resource { .. },
            )
            | (
                InterchangeProjectUsage::Resource { .. },
                InterchangeProjectUsage::Directory { .. },
            ) => false,
        }
    }
}

impl ResolutionInfo {
    pub fn new(usage: InterchangeProjectUsage, base_path: Option<Utf8PathBuf>) -> Self {
        Self { usage, base_path }
    }

    pub fn iri(iri: Iri<String>) -> Self {
        Self {
            usage: InterchangeProjectUsage::Resource {
                resource: iri,
                version_constraint: None,
            },
            base_path: None,
        }
    }

    pub fn usage(&self) -> &InterchangeProjectUsage {
        &self.usage
    }

    pub fn base_path(&self) -> Option<&Utf8Path> {
        self.base_path.as_deref()
    }

    /// Identifier of this usage, to be used in lock/env.
    // TODO: how to take versions/requirements into account here?
    pub fn id(&self) -> Identifier {
        Identifier::from_interchange_usage(&self.usage)
    }
}

impl Display for ResolutionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.usage {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint,
            } => {
                write!(f, "IRI `{resource}`")?;
                if let Some(vc) = version_constraint {
                    write!(f, " ({vc})")?;
                }
            }
            InterchangeProjectUsage::Directory {
                dir,
                publisher,
                name,
            } => {
                if let Some(bp) = &self.base_path {
                    let abs_path = bp.join(dir.as_str());
                    let abs_path = wrapfs::absolute(&abs_path).unwrap_or(abs_path);
                    writeln!(f, "`{publisher}/{name}` from `{abs_path}`",)?;
                } else {
                    writeln!(f, "`{publisher}/{name}` from `{dir}` (full path unknown)",)?;
                }
            }
        }
        write!(f, "{}", self.usage)?;
        if let Some(path) = &self.base_path {
            write!(f, ", base path `{path}`")
        } else {
            Ok(())
        }
    }
}

pub trait ResolveRead {
    type Error: ErrorBound;

    type ProjectStorage: ProjectRead;
    type ResolvedStorages: IntoIterator<Item = Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
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

    fn resolve_read_async(
        &self,
        resolve: &ResolutionInfo,
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
    //futures::stream::Iter<<<T as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter>;

    async fn resolve_read_async(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(match self.inner.resolve_read(resolve)? {
            ResolutionOutcome::Resolved(projects) => ResolutionOutcome::Resolved({
                let projects_map: std::iter::Map<_, fn(_) -> _> = projects
                    .into_iter()
                    .map(|proj| Ok(AsAsyncProject { inner: proj? }));

                futures::stream::iter(projects_map)
            }),
            ResolutionOutcome::UnsupportedUsageType { reason } => {
                ResolutionOutcome::UnsupportedUsageType { reason }
            }
            ResolutionOutcome::Unresolvable { reason } => {
                ResolutionOutcome::Unresolvable { reason }
            }
            ResolutionOutcome::NotFound { reason } => ResolutionOutcome::NotFound { reason },
        })
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
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(
            match self
                .runtime
                .block_on(self.inner.resolve_read_async(resolve))?
            {
                ResolutionOutcome::Resolved(storages) => {
                    let runtime_clone = self.runtime.clone();

                    let inner: futures::stream::Map<_, Box<dyn Fn(_) -> _>> =
                        storages.map(Box::new(move |project| {
                            Ok(AsSyncProjectTokio {
                                runtime: runtime_clone.clone(),
                                inner: project?,
                            })
                        }));

                    ResolutionOutcome::Resolved(SyncStreamIter {
                        runtime: self.runtime.clone(),
                        inner,
                    })
                }
                ResolutionOutcome::UnsupportedUsageType { reason } => {
                    ResolutionOutcome::UnsupportedUsageType { reason }
                }
                ResolutionOutcome::Unresolvable { reason } => {
                    ResolutionOutcome::Unresolvable { reason }
                }
                ResolutionOutcome::NotFound { reason } => ResolutionOutcome::NotFound { reason },
            },
        )
    }
}
