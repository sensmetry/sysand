// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! One index environment type over both transports, so a single
//! `SequentialResolver<EnvResolver<_>>` can hold HTTP and filesystem
//! indexes in the user's configured order. [`ReadEnvironmentAsync`] is
//! not dyn-compatible (RPITIT and generic `AsRef<str>` parameters), so
//! the dispatch is an enum, one arm per transport; the resolver-facing
//! project and its errors are enums the same way.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use camino::Utf8Path;
use futures::{future::Either, io::AsyncRead};
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    env::{
        ProjectChecksum, ProjectChecksumResult, ReadEnvironmentAsync,
        index::{IndexEnvironmentAsync, IndexEnvironmentError},
        local_index::{LocalIndexEnvironment, LocalIndexError},
    },
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        AsAsyncProject, CanonicalizationError, ProjectFormat, ProjectReadAsync,
        index_entry::{IndexEntryProject, IndexEntryProjectError},
        local_kpar::{LocalKParError, LocalKParProject},
    },
};

/// An index the resolver reads through either transport.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "one value per configured index; the HTTP arm's size is its caches, not waste"
)]
pub enum AnyIndexEnvironment<Policy> {
    Remote(IndexEnvironmentAsync<Policy>),
    Filesystem(LocalIndexEnvironment),
}

#[derive(Error, Debug)]
pub enum AnyIndexError {
    #[error(transparent)]
    Remote(#[from] IndexEnvironmentError),
    #[error(transparent)]
    Filesystem(#[from] LocalIndexError),
}

/// A project an index handed out, over either transport.
#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "one value per candidate version; the HTTP arm carries its download state"
)]
pub enum AnyIndexProject<Policy> {
    Remote(IndexEntryProject<Policy>),
    Filesystem(AsAsyncProject<LocalKParProject>),
}

#[derive(Error, Debug)]
pub enum AnyIndexProjectError {
    #[error(transparent)]
    Remote(#[from] IndexEntryProjectError),
    #[error(transparent)]
    Filesystem(#[from] LocalKParError),
}

fn canonical<E, F: crate::env::utils::ErrorBound + Into<AnyIndexProjectError>>(
    result: Result<E, CanonicalizationError<F>>,
) -> Result<E, CanonicalizationError<AnyIndexProjectError>> {
    result.map_err(|e| match e {
        CanonicalizationError::ProjectRead(inner) => {
            CanonicalizationError::ProjectRead(inner.into())
        }
        CanonicalizationError::FileRead(path, io) => CanonicalizationError::FileRead(path, io),
    })
}

/// The source reader of either arm; both are `Unpin` by the trait's
/// bound, so polling projects through a plain `Pin::new`.
pub enum AnyIndexSourceReader<'a, Policy: HTTPAuthentication + 'a> {
    Remote(<IndexEntryProject<Policy> as ProjectReadAsync>::SourceReader<'a>),
    Filesystem(<AsAsyncProject<LocalKParProject> as ProjectReadAsync>::SourceReader<'a>),
}

impl<Policy: HTTPAuthentication> AsyncRead for AnyIndexSourceReader<'_, Policy> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Remote(reader) => Pin::new(reader).poll_read(cx, buf),
            Self::Filesystem(reader) => Pin::new(reader).poll_read(cx, buf),
        }
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for AnyIndexProject<Policy> {
    type Error = AnyIndexProjectError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            Self::Remote(p) => p.get_project_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.get_project_async().await.map_err(Into::into),
        }
    }

    type SourceReader<'a>
        = AnyIndexSourceReader<'a, Policy>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(match self {
            Self::Remote(p) => AnyIndexSourceReader::Remote(p.read_source_async(path).await?),
            Self::Filesystem(p) => {
                AnyIndexSourceReader::Filesystem(p.read_source_async(path).await?)
            }
        })
    }

    async fn sources_async(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            Self::Remote(p) => p.sources_async(ctx).await.map_err(Into::into),
            Self::Filesystem(p) => p.sources_async(ctx).await.map_err(Into::into),
        }
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        match self {
            Self::Remote(p) => p.source_may_offer_multiple_versions(),
            Self::Filesystem(p) => p.source_may_offer_multiple_versions(),
        }
    }

    // Every optional method is forwarded so an arm's override (the
    // HTTP entry answers `version`/`usage` from `versions.json` without
    // a download; the archive arm honours prefetched digests) stays in
    // force behind the enum.
    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        match self {
            Self::Remote(p) => p.get_info_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.get_info_async().await.map_err(Into::into),
        }
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        match self {
            Self::Remote(p) => p.get_meta_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.get_meta_async().await.map_err(Into::into),
        }
    }

    async fn name_async(&self) -> Result<Option<String>, Self::Error> {
        match self {
            Self::Remote(p) => p.name_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.name_async().await.map_err(Into::into),
        }
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        match self {
            Self::Remote(p) => p.is_definitely_invalid_async().await,
            Self::Filesystem(p) => p.is_definitely_invalid_async().await,
        }
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        match self {
            Self::Remote(p) => p.version_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.version_async().await.map_err(Into::into),
        }
    }

    async fn usage_async(
        &self,
    ) -> Result<Option<Vec<crate::model::InterchangeProjectUsageRaw>>, Self::Error> {
        match self {
            Self::Remote(p) => p.usage_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.usage_async().await.map_err(Into::into),
        }
    }

    async fn checksum_async(
        &self,
    ) -> Result<
        Option<indexmap::IndexMap<String, crate::model::InterchangeProjectChecksumRaw>>,
        Self::Error,
    > {
        match self {
            Self::Remote(p) => p.checksum_async().await.map_err(Into::into),
            Self::Filesystem(p) => p.checksum_async().await.map_err(Into::into),
        }
    }

    async fn canonical_meta_async(
        &self,
    ) -> Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>> {
        match self {
            Self::Remote(p) => canonical(p.canonical_meta_async().await),
            Self::Filesystem(p) => canonical(p.canonical_meta_async().await),
        }
    }

    async fn checksum_non_canonical_hex_async(&self) -> Result<Option<String>, Self::Error> {
        match self {
            Self::Remote(p) => p
                .checksum_non_canonical_hex_async()
                .await
                .map_err(Into::into),
            Self::Filesystem(p) => p
                .checksum_non_canonical_hex_async()
                .await
                .map_err(Into::into),
        }
    }

    async fn checksum_canonical_hex_async(
        &self,
    ) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        match self {
            Self::Remote(p) => canonical(p.checksum_canonical_hex_async().await),
            Self::Filesystem(p) => canonical(p.checksum_canonical_hex_async().await),
        }
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        match self {
            Self::Remote(p) => p
                .checksum_canonical_variant_async()
                .await
                .map_err(Into::into),
            Self::Filesystem(p) => p
                .checksum_canonical_variant_async()
                .await
                .map_err(Into::into),
        }
    }

    fn project_root(&self) -> Option<&Utf8Path> {
        match self {
            Self::Remote(p) => p.project_root(),
            Self::Filesystem(p) => p.project_root(),
        }
    }

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        match self {
            Self::Remote(p) => p.format().map_err(Into::into),
            Self::Filesystem(p) => p.format().map_err(Into::into),
        }
    }
}

type MapErr<S, F> = futures::stream::MapErr<S, F>;
type RemoteStream<S> = MapErr<S, fn(IndexEnvironmentError) -> AnyIndexError>;
type FilesystemStream<S> = MapErr<S, fn(LocalIndexError) -> AnyIndexError>;

impl<Policy: HTTPAuthentication> ReadEnvironmentAsync for AnyIndexEnvironment<Policy> {
    type ReadError = AnyIndexError;

    type UriStream = Either<
        RemoteStream<<IndexEnvironmentAsync<Policy> as ReadEnvironmentAsync>::UriStream>,
        FilesystemStream<<LocalIndexEnvironment as ReadEnvironmentAsync>::UriStream>,
    >;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        use futures::TryStreamExt as _;
        Ok(match self {
            Self::Remote(env) => Either::Left(
                env.uris_async()
                    .await?
                    .map_err(AnyIndexError::Remote as fn(_) -> _),
            ),
            Self::Filesystem(env) => Either::Right(
                env.uris_async()
                    .await?
                    .map_err(AnyIndexError::Filesystem as fn(_) -> _),
            ),
        })
    }

    type VersionStream = Either<
        RemoteStream<<IndexEnvironmentAsync<Policy> as ReadEnvironmentAsync>::VersionStream>,
        FilesystemStream<<LocalIndexEnvironment as ReadEnvironmentAsync>::VersionStream>,
    >;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        use futures::TryStreamExt as _;
        Ok(match self {
            Self::Remote(env) => Either::Left(
                env.versions_async(uri)
                    .await?
                    .map_err(AnyIndexError::Remote as fn(_) -> _),
            ),
            Self::Filesystem(env) => Either::Right(
                env.versions_async(uri)
                    .await?
                    .map_err(AnyIndexError::Filesystem as fn(_) -> _),
            ),
        })
    }

    type InterchangeProjectRead = AnyIndexProject<Policy>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(match self {
            Self::Remote(env) => {
                AnyIndexProject::Remote(env.get_project_async(uri, version).await?)
            }
            Self::Filesystem(env) => {
                AnyIndexProject::Filesystem(env.get_project_async(uri, version).await?)
            }
        })
    }

    async fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError> {
        match self {
            Self::Remote(env) => env
                .has_version_verified_async(uri, version, checksum)
                .await
                .map_err(Into::into),
            Self::Filesystem(env) => env
                .has_version_verified_async(uri, version, checksum)
                .await
                .map_err(Into::into),
        }
    }
}
