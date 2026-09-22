// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{fmt::Debug, marker::Unpin, sync::Arc};

use futures::{Stream, StreamExt as _, TryStreamExt as _};
use thiserror::Error;

use crate::{
    cancel::{Cancellable, CancellationToken, bridge},
    env::utils::ErrorBound,
    project::{
        AsAsyncProject, AsSyncProjectTokio, ProjectChecksum, ProjectMut, ProjectRead,
        ProjectReadAsync,
    },
};

// Implementations
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod any_index;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod discovery;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod index;
#[cfg(feature = "filesystem")]
pub mod local_directory;
#[cfg(feature = "filesystem")]
pub mod local_index;
pub mod memory;
pub mod null;

pub mod utils;

pub const DEFAULT_ENV_NAME: &str = ".sysand";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProjectChecksumResult {
    /// Version is not present. This can also mean no versions
    /// of the project are present, i.e. project itself was not
    /// found
    VersionNotFound,
    /// Checksum is not present for the version
    ChecksumNotPresent,
    /// Checksum kinds differ between requested and actual
    DifferentChecksumKinds,
    /// Checksum values do not match
    Mismatch,
    Match,
}

pub trait ReadEnvironment {
    type ReadError: ErrorBound;

    type UriIter: IntoIterator<Item = Result<String, Self::ReadError>>;
    /// Get URIs of all projects stored in the environment, each project
    /// once however many of its versions are installed. If a project has
    /// multiple URIs, only the first one is returned
    fn uris(&self) -> Result<Self::UriIter, Self::ReadError>;

    type VersionIter: IntoIterator<Item = Result<String, Self::ReadError>>;
    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError>;

    type InterchangeProjectRead: ProjectRead + Debug;
    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError>;

    // Utilities

    fn has<S: AsRef<str>>(&self, uri: S) -> Result<bool, Self::ReadError> {
        for res in self.uris()? {
            let u = res?;
            if u == uri.as_ref() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn has_version<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<bool, Self::ReadError> {
        for res in self.versions(&uri)? {
            let v = res?;
            if v == version.as_ref() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // TODO: decide on whtat should be returned here in these cases:
    // - project not present: Ok(VersionNotFound)
    // - project version not present: Ok(VersionNotFound) - both cases return VersionNotFound
    //   to give freedom to implementers; also to callers it makes no difference whether
    //   project or specific version was not found
    // - checksum does not match: Ok(Mismatch)
    // - checksum matches: Ok(Match)
    fn has_version_verified<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError>;

    fn candidate_projects<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Vec<Self::InterchangeProjectRead>, Self::ReadError> {
        let versions: Result<Vec<_>, _> = self.versions(&uri)?.into_iter().collect();

        let projects: Result<Vec<_>, _> = versions?
            .into_iter()
            .map(|v| self.get_project(&uri, v))
            .collect();

        projects
    }

    /// Treat this `ReadEnvironment` as a (trivial) `ReadEnvironmentAsync`
    fn to_async(self) -> AsAsyncEnvironment<Self>
    where
        Self: Sized,
    {
        AsAsyncEnvironment { inner: self }
    }
}

pub trait ReadEnvironmentAsync {
    type ReadError: ErrorBound;

    type UriStream: futures::Stream<Item = Result<String, Self::ReadError>>;
    /// Get URIs of all projects stored in the environment. If a project
    /// has multiple URIs, only the first one is returned
    fn uris_async(&self) -> impl Future<Output = Result<Self::UriStream, Self::ReadError>>;

    type VersionStream: futures::Stream<Item = Result<String, Self::ReadError>>;
    fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> impl Future<Output = Result<Self::VersionStream, Self::ReadError>>;

    type InterchangeProjectRead: ProjectReadAsync + Debug;
    fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> impl Future<Output = Result<Self::InterchangeProjectRead, Self::ReadError>>;

    // Utilities

    fn has_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> impl Future<Output = Result<bool, Self::ReadError>> {
        async move {
            let needle = uri.as_ref();
            let uris = self.uris_async().await?;
            uris.try_any(|u| async move { u == needle }).await
        }
    }

    fn has_version_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> impl Future<Output = Result<bool, Self::ReadError>> {
        async move {
            let needle = version.as_ref();
            let versions = self.versions_async(&uri).await?;
            versions.try_any(|v| async move { v == needle }).await
        }
    }

    fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> impl Future<Output = Result<ProjectChecksumResult, Self::ReadError>>;

    fn candidate_projects_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> impl Future<Output = Result<Vec<Self::InterchangeProjectRead>, Self::ReadError>> {
        async move {
            futures::future::join_all(
                self.versions_async(&uri)
                    .await?
                    .map(async |v| self.get_project_async(&uri, v?).await)
                    .collect::<Vec<_>>()
                    .await,
            )
            .await
            .into_iter()
            .collect()
        }
    }

    // TODO: Maybe make this return an associated type instead? Would, for example, allow
    // .as_async.as_tokio_sync == .as_tokio_sync.as_async == id
    /// Treat this `ReadEnvironmentAsync` as a `ReadEnvironment`, using the supplied tokio runtime.
    fn to_tokio_sync(self, runtime: Arc<tokio::runtime::Runtime>) -> AsSyncEnvironmentTokio<Self>
    where
        Self: Sized,
    {
        AsSyncEnvironmentTokio::new(runtime, self)
    }

    /// [`Self::to_tokio_sync`] observing `token` at every bridge.
    fn to_tokio_sync_with(
        self,
        runtime: Arc<tokio::runtime::Runtime>,
        token: CancellationToken,
    ) -> AsSyncEnvironmentTokio<Self>
    where
        Self: Sized,
    {
        AsSyncEnvironmentTokio::new(runtime, self).with_cancellation(token)
    }
}

/// Intended to wrap a `ReadEnvironment`, turning it into a (trivial)
/// `ReadEnvironmentAsync`.
#[derive(Debug)]
pub struct AsAsyncEnvironment<T> {
    inner: T,
}

impl<T: ReadEnvironment> ReadEnvironmentAsync for AsAsyncEnvironment<T>
where
    for<'a> <<T as ReadEnvironment>::InterchangeProjectRead as ProjectRead>::SourceReader<'a>:
        Unpin,
{
    type ReadError = <T as ReadEnvironment>::ReadError;

    type UriStream =
        futures::stream::Iter<<<T as ReadEnvironment>::UriIter as IntoIterator>::IntoIter>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        Ok(futures::stream::iter(self.inner.uris()?))
    }

    type VersionStream =
        futures::stream::Iter<<<T as ReadEnvironment>::VersionIter as IntoIterator>::IntoIter>;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        Ok(futures::stream::iter(self.inner.versions(uri)?))
    }

    type InterchangeProjectRead = AsAsyncProject<<T as ReadEnvironment>::InterchangeProjectRead>;

    async fn get_project_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(AsAsyncProject {
            inner: self.inner.get_project(uri, version)?,
        })
    }

    async fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError> {
        self.inner.has_version_verified(uri, version, checksum)
    }
}

/// Wrapper intended to wrap an `ReadEnvironmentAsync` as a `ReadEnvironment`
/// using a provided tokio runtime; every bridge observes the cancellation
/// token (`crate::cancel`).
#[derive(Debug)]
#[non_exhaustive]
pub struct AsSyncEnvironmentTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
    token: CancellationToken,
}

impl<T> AsSyncEnvironmentTokio<T> {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, inner: T) -> Self {
        Self {
            runtime,
            inner,
            token: CancellationToken::new(),
        }
    }

    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }
}

/// A `Stream` of results driven synchronously on a tokio runtime. A
/// cancellation is yielded once, as `Err(Cancelled)`, and the iteration
/// then ends.
#[derive(Debug)]
#[non_exhaustive]
pub struct SyncStreamIter<S> {
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub inner: S,
    pub token: CancellationToken,
    done: bool,
}

impl<S> SyncStreamIter<S> {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, inner: S) -> Self {
        Self {
            runtime,
            inner,
            token: CancellationToken::new(),
            done: false,
        }
    }

    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }
}

impl<S, T, E> Iterator for SyncStreamIter<S>
where
    S: Stream<Item = Result<T, E>> + Unpin,
{
    type Item = Result<T, Cancellable<E>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Self {
            runtime,
            inner,
            token,
            done,
        } = self;
        match runtime.block_on(token.run_until_cancelled(inner.next())) {
            Some(Some(item)) => Some(item.map_err(Cancellable::Inner)),
            Some(None) => {
                *done = true;
                None
            }
            None => {
                *done = true;
                Some(Err(Cancellable::Cancelled))
            }
        }
    }
}

impl<T: ReadEnvironmentAsync> ReadEnvironment for AsSyncEnvironmentTokio<T>
where
    <T as ReadEnvironmentAsync>::UriStream: Unpin,
    <T as ReadEnvironmentAsync>::VersionStream: Unpin,
{
    type ReadError = Cancellable<<T as ReadEnvironmentAsync>::ReadError>;

    type UriIter = SyncStreamIter<<T as ReadEnvironmentAsync>::UriStream>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let stream = bridge(&self.runtime, &self.token, self.inner.uris_async())?;
        Ok(SyncStreamIter::new(self.runtime.clone(), stream).with_cancellation(self.token.clone()))
    }

    type VersionIter = SyncStreamIter<<T as ReadEnvironmentAsync>::VersionStream>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let stream = bridge(&self.runtime, &self.token, self.inner.versions_async(uri))?;
        Ok(SyncStreamIter::new(self.runtime.clone(), stream).with_cancellation(self.token.clone()))
    }

    type InterchangeProjectRead =
        AsSyncProjectTokio<<T as ReadEnvironmentAsync>::InterchangeProjectRead>;

    fn get_project<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let project = bridge(
            &self.runtime,
            &self.token,
            self.inner.get_project_async(uri, version),
        )?;
        Ok(AsSyncProjectTokio::new(self.runtime.clone(), project)
            .with_cancellation(self.token.clone()))
    }

    fn has_version_verified<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError> {
        bridge(
            &self.runtime,
            &self.token,
            self.inner
                .has_version_verified_async(uri, version, checksum),
        )
    }
}

#[derive(Error, Debug)]
pub enum PutProjectError<WE, CE> {
    #[error(transparent)]
    Write(#[from] WE),
    #[error(transparent)]
    Callback(CE),
    #[error("failed to parse `{0}` as IRI: {1}")]
    IriParse(String, fluent_uri::ParseError),
    // #[error(transparent)]
    // AddProject(AE),
}

pub trait WriteEnvironment {
    type WriteError: ErrorBound;

    type InterchangeProjectMut: ProjectMut;

    // TODO: Should this be replaced by a transactional interface?
    /// Install a project in the environment. The project files will be copied into
    /// the environment, so this is not suitable for editable projects
    fn put_project<S: AsRef<str>, T: AsRef<str>, F, CE>(
        &mut self,
        uri: S,
        version: T,
        checksum: Option<ProjectChecksum>,
        // Callback allows the implementation to gracefully recover
        // in case of an error, to just "allocate"
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, CE>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), CE>;

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError>;

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError>;
}
