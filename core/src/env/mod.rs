// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fmt::Debug, marker::Unpin, sync::Arc};

use futures::{Stream, StreamExt};
use sha2::Digest;

use thiserror::Error;

use crate::project::{
    AsAsyncProject, AsSyncProjectTokio, ProjectMut, ProjectRead, ProjectReadAsync,
};

// pub mod utils;

// Implementations
#[cfg(feature = "filesystem")]
pub mod local_directory;
pub mod memory;
pub mod null;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod reqwest_http;

pub mod utils;

/// Get path segment(s) correspoding to the given `uri`
pub fn segment_uri_generic<S: AsRef<str>, D: Digest>(uri: S) -> std::vec::IntoIter<String>
where
    digest::Output<D>: core::fmt::LowerHex,
{
    let mut hasher = D::new();
    hasher.update(uri.as_ref());

    vec![format!("{:x}", hasher.finalize())].into_iter()
}

pub trait ReadEnvironment {
    type ReadError: std::error::Error + Debug;

    type UriIter: IntoIterator<Item = Result<String, Self::ReadError>>;
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
        Ok(self
            .uris()?
            .into_iter()
            .filter_map(Result::ok)
            .any(|u: String| u == uri.as_ref()))
    }

    fn has_version<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<bool, Self::ReadError> {
        Ok(self
            .versions(&uri)?
            .into_iter()
            .filter_map(Result::ok)
            .any(|v: String| v == version.as_ref()))
    }

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
    type ReadError: std::error::Error + Debug;

    type UriStream: futures::Stream<Item = Result<String, Self::ReadError>>;
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
            let uri = uri.as_ref();

            Ok(self
                .uris_async()
                .await?
                .filter_map(|x| async move { Result::ok(x) })
                .any(|u: String| async move { u == uri })
                .await)
        }
    }

    fn has_version_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> impl Future<Output = Result<bool, Self::ReadError>> {
        async move {
            let version = version.as_ref();

            Ok(self
                .versions_async(&uri)
                .await?
                .filter_map(|x| async move { Result::ok(x) })
                .any(|v: String| async move { v == version })
                .await)
        }
    }

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
        AsSyncEnvironmentTokio {
            runtime,
            inner: self,
        }
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
}

/// Wrapper intended to wrap an `ReadEnvironmentAsync` as a `ReadEnvironment`
/// using a provided tokio runtime.
#[derive(Debug)]
pub struct AsSyncEnvironmentTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
}

#[derive(Debug)]
pub struct SyncStreamIter<S> {
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub inner: S,
}

impl<S: Stream + Unpin> Iterator for SyncStreamIter<S> {
    type Item = <S as Stream>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.runtime.block_on(self.inner.next())
    }
}

impl<T: ReadEnvironmentAsync> ReadEnvironment for AsSyncEnvironmentTokio<T>
where
    <T as ReadEnvironmentAsync>::UriStream: Unpin,
    <T as ReadEnvironmentAsync>::VersionStream: Unpin,
{
    type ReadError = <T as ReadEnvironmentAsync>::ReadError;

    type UriIter = SyncStreamIter<<T as ReadEnvironmentAsync>::UriStream>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let stream = self.runtime.block_on(self.inner.uris_async())?;

        Ok(SyncStreamIter {
            runtime: self.runtime.clone(),
            inner: stream,
        })
    }

    type VersionIter = SyncStreamIter<<T as ReadEnvironmentAsync>::VersionStream>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let stream = self.runtime.block_on(self.inner.versions_async(uri))?;

        Ok(SyncStreamIter {
            runtime: self.runtime.clone(),
            inner: stream,
        })
    }

    type InterchangeProjectRead =
        AsSyncProjectTokio<<T as ReadEnvironmentAsync>::InterchangeProjectRead>;

    fn get_project<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(AsSyncProjectTokio {
            runtime: self.runtime.clone(),
            inner: self
                .runtime
                .block_on(self.inner.get_project_async(uri, version))?,
        })
    }
}

#[derive(Error, Debug)]
pub enum PutProjectError<WE, CE> {
    #[error(transparent)]
    Write(#[from] WE),
    #[error(transparent)]
    Callback(CE),
}

pub trait WriteEnvironment {
    type WriteError: std::error::Error + Debug;

    type InterchangeProjectMut: ProjectMut;

    // TODO: Should this be replaced by a transactional interface?
    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        // Callback allows the implementation to gracefully recover
        // in case of an error, to just "allocate"
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>;

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError>;

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError>;
}
