// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8Path;
use digest::{array::Array, typenum};
use futures::io::{AsyncBufReadExt as _, AsyncRead};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use std::{
    fmt::{Debug, Display},
    io::{self, BufRead as _, BufReader, Read},
    marker::Unpin,
    num::NonZeroU64,
    sync::Arc,
};
use thiserror::Error;
use utils::FsIoError;

pub use sysand_macros::ProjectMut;
pub use sysand_macros::ProjectRead;
pub use typed_path::Utf8UnixPath;

use crate::{
    context::ProjectContext,
    env::utils::ErrorBound,
    lock::Source,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw, KerMlChecksumAlg, ProjectHash, project_hash_hex,
    },
    utils::lowercase_hex,
};

// Implementations
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod any;
pub mod editable;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod gix_git_download;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod index_entry;
#[cfg(feature = "filesystem")]
pub mod local_kpar;
#[cfg(feature = "filesystem")]
pub mod local_src;
pub mod memory;
pub mod null;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod reqwest_kpar_download;
// TODO: Reintroduce this module
// #[cfg(all(feature = "filesystem", feature = "networking"))]
// pub mod reqwest_kpar_ranged;
#[cfg(feature = "networking")]
pub mod reqwest_src;

// Generic implementations
pub mod cached;
pub mod reference;

pub mod utils;

#[derive(Debug)]
pub struct KparMeta {
    pub size_bytes: NonZeroU64,
    pub sha256_hex: String,
}

/// Produce a SHA-256 digest by hashing all the contents of `reader`
pub(crate) fn hash_reader<R: Read>(reader: &mut R) -> Result<Array<u8, typenum::U32>, io::Error> {
    let mut hasher = Sha256::new();
    let mut buffered = BufReader::new(reader);

    loop {
        let buffer = buffered.fill_buf()?;
        let length = buffer.len();

        if length == 0 {
            break;
        }

        hasher.update(buffer);

        buffered.consume(length);
    }

    Ok(hasher.finalize())
}

async fn hash_reader_async<R: AsyncRead + Unpin>(reader: &mut R) -> Result<ProjectHash, io::Error> {
    let mut hasher = Sha256::new();
    let mut buffered = futures::io::BufReader::new(reader);

    loop {
        let buffer = buffered.fill_buf().await?;
        let length = buffer.len();

        if length == 0 {
            break;
        }

        hasher.update(buffer);

        buffered.consume_unpin(length);
    }

    Ok(hasher.finalize())
}

#[derive(Error, Debug)]
pub enum CanonicalizationError<ReadError: ErrorBound> {
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error("failed to read from file\n  `{0}`:\n  {1}")]
    FileRead(Box<str>, io::Error),
}

impl<E: ErrorBound> CanonicalizationError<E> {
    /// Map the inner `ProjectRead` error type while leaving the `FileRead`
    /// variant untouched. Convenient when forwarding a canonicalization
    /// result across a wrapper type that rewraps the underlying read error.
    pub fn map_project_read<F, E2>(self, f: F) -> CanonicalizationError<E2>
    where
        F: FnOnce(E) -> E2,
        E2: ErrorBound,
    {
        match self {
            Self::ProjectRead(e) => CanonicalizationError::ProjectRead(f(e)),
            Self::FileRead(p, io) => CanonicalizationError::FileRead(p, io),
        }
    }
}

#[derive(Debug, Error)]
pub enum IntoProjectError<ReadError: ErrorBound, W: ProjectMut> {
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error(transparent)]
    ProjectWrite(W::Error),
    #[error("missing project information file `.project.json`")]
    MissingInfo,
    #[error("missing project metadata file `.meta.json`")]
    MissingMeta,
}

// TODO: serialize this as "kpar:<hex>" or "meta:<hex>" to avoid having two fields,
// which can't be both populated
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectChecksum {
    /// Same as `.checksum_canonical_hex()`
    Project(String),
    /// SHA256 hex digest of the original KPAR
    Kpar(String),
}

impl Display for ProjectChecksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectChecksum::Project(c) => write!(f, "kpar:{c}"),
            ProjectChecksum::Kpar(c) => write!(f, "src:{c}"),
        }
    }
}

/// Anything implementing `ProjectRead` can be treated as a method for accessing (one
/// particular) interchange project.
pub trait ProjectRead {
    // Mandatory

    type Error: ErrorBound;

    /// Fetch project information and metadata (if they exist).
    // TODO: cache project info
    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    >;

    type SourceReader<'a>: Read
    where
        Self: 'a;

    /// Produces a `Read`er for the source file with path `path`
    /// inside a project. *May* require significant network activity.
    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error>;

    /// List (known) sources of this package. Typically
    /// this is a singleton, but may list multiple. In case
    /// multiple ones are listed they should aim to be in
    /// some typical order of preference.
    ///
    /// In case no sources are included, they should be derived
    /// from the known info, including `ctx` if possible.
    ///
    /// Must not return an empty list; should panic if no sources are available.
    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error>;

    // Optional and helpers

    /// Returns the local filesystem root path of this project, if available.
    /// It is used, among other things, to resolve relative path usages. Such
    /// usages will fail resolution if `None` is returned here
    fn project_root(&self) -> Option<&Utf8Path> {
        None
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(self.get_project()?.0)
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(self.get_project()?.1)
    }

    fn publisher(&self) -> Result<Option<Option<String>>, Self::Error> {
        Ok(self.get_info()?.map(|info| info.publisher))
    }

    fn name(&self) -> Result<Option<String>, Self::Error> {
        Ok(self.get_info()?.map(|info| info.name))
    }

    /// `is_definitely_invalid` will return `true` only if get_project() would definitely
    /// produce an error or return `Some((info, meta))` where either `info` or `meta`
    /// are `None`. If it returns `false` nothing definite can be said.
    ///
    /// Implementations may use this to give shortcuts for eliminating potential interchange
    /// projects. *Should* be significantly faster than running `get_project`.
    fn is_definitely_invalid(&self) -> bool {
        false
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        Ok(self.get_info()?.map(|info| info.version))
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(self.get_info()?.map(|info| info.usage))
    }

    fn checksum(
        &self,
    ) -> Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error> {
        Ok(self.get_meta()?.and_then(|meta| meta.checksum))
    }

    /// Produces canonicalized project metadata, replacing all source file hashes by SHA256.
    fn canonical_meta(
        &self,
    ) -> Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>> {
        let Some(mut meta) = self
            .get_meta()
            .map_err(CanonicalizationError::ProjectRead)?
        else {
            return Ok(None);
        };

        for (path, checksum) in meta
            .checksum
            .as_mut()
            .into_iter()
            .flat_map(|index| index.iter_mut())
        {
            let sha256: &str = KerMlChecksumAlg::Sha256.into();
            if checksum.algorithm != sha256 {
                checksum.algorithm = sha256.to_owned();

                let mut src = self
                    .read_source(path)
                    .map_err(CanonicalizationError::ProjectRead)?;
                checksum.value = lowercase_hex(
                    hash_reader(&mut src)
                        .map_err(|e| CanonicalizationError::FileRead(path.as_str().into(), e))?,
                );
            } else {
                checksum.value = checksum.value.to_lowercase();
            }
        }

        Ok(Some(meta))
    }

    /// Produces a project hash based on project information and the *non-canonicalized* metadata.
    fn checksum_non_canonical_hex(&self) -> Result<Option<String>, Self::Error> {
        Ok(self
            .get_project()
            .map(|(info, meta)| info.zip(meta))?
            .map(|(info, meta)| project_hash_hex(&info, &meta)))
    }

    /// Produces a project hash based on project information and the
    /// *canonicalized* metadata.
    ///
    /// `Ok(None)` means the project has no `.project.json` or no
    /// `.meta.json` — one of the two required inputs is absent — and
    /// callers can rely on that meaning rather than treating `None` as
    /// an unspecified failure mode.
    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        let info = self
            .get_info()
            .map_err(CanonicalizationError::ProjectRead)?;
        let meta = self.canonical_meta()?;

        Ok(info
            .zip(meta)
            .map(|(info, meta)| project_hash_hex(&info, &meta)))
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error>;

    // TODO: Make this return an associated type instead?
    /// Treat this `ProjectRead` as a (trivial) `ProjectReadAsync`
    fn to_async(self) -> AsAsyncProject<Self>
    where
        Self: Sized,
    {
        AsAsyncProject { inner: self }
    }
}

impl<T: ProjectRead> ProjectRead for &T {
    type Error = T::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        (*self).get_project()
    }

    type SourceReader<'a>
        = T::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        (*self).read_source(path)
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        (*self).sources(ctx)
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        (*self).get_info()
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        (*self).get_meta()
    }

    fn name(&self) -> Result<Option<String>, Self::Error> {
        (*self).name()
    }

    fn is_definitely_invalid(&self) -> bool {
        (*self).is_definitely_invalid()
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        (*self).version()
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        (*self).usage()
    }

    fn checksum(
        &self,
    ) -> Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error> {
        (*self).checksum()
    }

    fn canonical_meta(
        &self,
    ) -> Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>> {
        (*self).canonical_meta()
    }

    fn checksum_non_canonical_hex(&self) -> Result<Option<String>, Self::Error> {
        (*self).checksum_non_canonical_hex()
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        (*self).checksum_canonical_hex()
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        (*self).checksum_canonical_variant()
    }
}

impl<T: ProjectRead> ProjectRead for &mut T {
    type Error = T::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        (**self).get_project()
    }

    type SourceReader<'a>
        = T::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        (**self).read_source(path)
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        (**self).sources(ctx)
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        (**self).get_info()
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        (**self).get_meta()
    }

    fn name(&self) -> Result<Option<String>, Self::Error> {
        (**self).name()
    }

    fn is_definitely_invalid(&self) -> bool {
        (**self).is_definitely_invalid()
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        (**self).version()
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        (**self).usage()
    }

    fn checksum(
        &self,
    ) -> Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error> {
        (**self).checksum()
    }

    fn canonical_meta(
        &self,
    ) -> Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>> {
        (**self).canonical_meta()
    }

    fn checksum_non_canonical_hex(&self) -> Result<Option<String>, Self::Error> {
        (**self).checksum_non_canonical_hex()
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        (**self).checksum_canonical_hex()
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        (**self).checksum_canonical_variant()
    }
}

pub trait ProjectReadAsync {
    // Mandatory

    type Error: ErrorBound;

    /// Fetch project information and metadata (if they exist).
    fn get_project_async(
        &self,
    ) -> impl Future<
        Output = Result<
            (
                Option<InterchangeProjectInfoRaw>,
                Option<InterchangeProjectMetadataRaw>,
            ),
            Self::Error,
        >,
    >;

    type SourceReader<'a>: AsyncRead + Unpin
    where
        Self: 'a;

    /// Produces a `Read`er for the source file with path `path`
    /// inside a project. *May* require significant network activity.
    fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> impl Future<Output = Result<Self::SourceReader<'_>, Self::Error>>;

    /// List (known) sources of this package. Typically
    /// this is a singleton, but may list multiple. In case
    /// multiple ones are listed they should aim to be in
    /// some typical order of preference.
    ///
    /// May be empty if no valid sources are known.
    // TODO: should we require the checksum to be verified (i.e. actual, instead of
    // expected)
    fn sources_async(
        &self,
        ctx: &ProjectContext,
    ) -> impl Future<Output = Result<Vec<Source>, Self::Error>>;

    // Optional and helpers

    fn get_info_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectInfoRaw>, Self::Error>> {
        async { Ok(self.get_project_async().await?.0) }
    }

    fn get_meta_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectMetadataRaw>, Self::Error>> {
        async { Ok(self.get_project_async().await?.1) }
    }

    fn name_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async { Ok(self.get_info_async().await?.map(|info| info.name)) }
    }

    /// `is_definitely_invalid` will return `true` only if `get_project()` would definitely
    /// produce an error or return `Some((info, meta))` where either `info` or `meta`
    /// are `None`. If it returns `false` nothing definite can be said.
    ///
    /// Implementations may use this to give shortcuts for eliminating potential interchange
    /// projects. *Should* be significantly faster than running `get_project`.
    fn is_definitely_invalid_async(&self) -> impl Future<Output = bool> {
        async { false }
    }

    fn version_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async { Ok(self.get_info_async().await?.map(|info| info.version)) }
    }

    fn usage_async(
        &self,
    ) -> impl Future<Output = Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error>> {
        async { Ok(self.get_info_async().await?.map(|info| info.usage)) }
    }

    fn checksum_async(
        &self,
    ) -> impl Future<Output = Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error>>
    {
        async { Ok(self.get_meta_async().await?.and_then(|meta| meta.checksum)) }
    }

    /// Produces canonicalized project metadata, replacing all source file hashes by SHA256.
    fn canonical_meta_async(
        &self,
    ) -> impl Future<
        Output = Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>>,
    > {
        async move {
            let Some(mut meta) = self
                .get_meta_async()
                .await
                .map_err(CanonicalizationError::ProjectRead)?
            else {
                return Ok(None);
            };

            if let Some(mut checksums) = meta.checksum {
                let future_checksums = checksums.drain(..).map(|(path, mut checksum)| async move {
                    let sha256: &str = KerMlChecksumAlg::Sha256.into();
                    if checksum.algorithm != sha256 {
                        checksum.algorithm = sha256.to_owned();

                        let mut src = self
                            .read_source_async(&path)
                            .await
                            .map_err(CanonicalizationError::ProjectRead)?;
                        checksum.value =
                            lowercase_hex(hash_reader_async(&mut src).await.map_err(|e| {
                                CanonicalizationError::FileRead(path.clone().into(), e)
                            })?);
                    } else {
                        checksum.value = checksum.value.to_lowercase();
                    }

                    Ok((path, checksum))
                });

                let collected_checksums: Result<Vec<(String, InterchangeProjectChecksumRaw)>, _> =
                    futures::future::join_all(future_checksums)
                        .await
                        .into_iter()
                        .collect();

                meta.checksum = Some(indexmap::IndexMap::from_iter(collected_checksums?));
            }

            Ok(Some(meta))
        }
    }

    /// Produces a project hash based on project information and the *non-canonicalized* metadata.
    fn checksum_non_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async {
            Ok(self
                .get_project_async()
                .await
                .map(|(info, meta)| info.zip(meta))?
                .map(|(info, meta)| project_hash_hex(&info, &meta)))
        }
    }

    /// Produces a project hash based on project information and the
    /// *canonicalized* metadata.
    fn checksum_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, CanonicalizationError<Self::Error>>> {
        async {
            let info = self
                .get_info_async()
                .await
                .map_err(CanonicalizationError::ProjectRead)?;
            let meta = self.canonical_meta_async().await?;

            Ok(info
                .zip(meta)
                .map(|(info, meta)| project_hash_hex(&info, &meta)))
        }
    }

    fn checksum_canonical_variant_async(
        &self,
    ) -> impl Future<Output = Result<ProjectChecksum, Self::Error>>;

    /// Treat this `ProjectReadAsync` as a `ProjectRead` using the provided tokio runtime.
    fn to_tokio_sync(self, runtime: Arc<tokio::runtime::Runtime>) -> AsSyncProjectTokio<Self>
    where
        Self: Sized,
    {
        AsSyncProjectTokio {
            runtime,
            inner: self,
        }
    }
}

impl<T: ProjectReadAsync> ProjectReadAsync for &T {
    type Error = T::Error;

    fn get_project_async(
        &self,
    ) -> impl Future<
        Output = Result<
            (
                Option<InterchangeProjectInfoRaw>,
                Option<InterchangeProjectMetadataRaw>,
            ),
            Self::Error,
        >,
    > {
        (**self).get_project_async()
    }

    type SourceReader<'a>
        = T::SourceReader<'a>
    where
        Self: 'a;

    fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> impl Future<Output = Result<Self::SourceReader<'_>, Self::Error>> {
        (**self).read_source_async(path)
    }

    fn sources_async(
        &self,
        ctx: &ProjectContext,
    ) -> impl Future<Output = Result<Vec<Source>, Self::Error>> {
        (**self).sources_async(ctx)
    }

    fn get_info_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectInfoRaw>, Self::Error>> {
        (**self).get_info_async()
    }

    fn get_meta_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectMetadataRaw>, Self::Error>> {
        (**self).get_meta_async()
    }

    fn name_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).name_async()
    }

    fn is_definitely_invalid_async(&self) -> impl Future<Output = bool> {
        (**self).is_definitely_invalid_async()
    }

    fn version_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).version_async()
    }

    fn usage_async(
        &self,
    ) -> impl Future<Output = Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error>> {
        (**self).usage_async()
    }

    fn checksum_async(
        &self,
    ) -> impl Future<Output = Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error>>
    {
        (**self).checksum_async()
    }

    fn canonical_meta_async(
        &self,
    ) -> impl Future<
        Output = Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>>,
    > {
        (**self).canonical_meta_async()
    }

    fn checksum_non_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).checksum_non_canonical_hex_async()
    }

    fn checksum_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, CanonicalizationError<Self::Error>>> {
        (**self).checksum_canonical_hex_async()
    }

    fn checksum_canonical_variant_async(
        &self,
    ) -> impl Future<Output = Result<ProjectChecksum, Self::Error>> {
        (**self).checksum_canonical_variant_async()
    }
}

impl<T: ProjectReadAsync> ProjectReadAsync for &mut T {
    type Error = T::Error;

    fn get_project_async(
        &self,
    ) -> impl Future<
        Output = Result<
            (
                Option<InterchangeProjectInfoRaw>,
                Option<InterchangeProjectMetadataRaw>,
            ),
            Self::Error,
        >,
    > {
        (**self).get_project_async()
    }

    type SourceReader<'a>
        = T::SourceReader<'a>
    where
        Self: 'a;

    fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> impl Future<Output = Result<Self::SourceReader<'_>, Self::Error>> {
        (**self).read_source_async(path)
    }

    fn sources_async(
        &self,
        ctx: &ProjectContext,
    ) -> impl Future<Output = Result<Vec<Source>, Self::Error>> {
        (**self).sources_async(ctx)
    }

    fn get_info_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectInfoRaw>, Self::Error>> {
        (**self).get_info_async()
    }

    fn get_meta_async(
        &self,
    ) -> impl Future<Output = Result<Option<InterchangeProjectMetadataRaw>, Self::Error>> {
        (**self).get_meta_async()
    }

    fn name_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).name_async()
    }

    fn is_definitely_invalid_async(&self) -> impl Future<Output = bool> {
        (**self).is_definitely_invalid_async()
    }

    fn version_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).version_async()
    }

    fn usage_async(
        &self,
    ) -> impl Future<Output = Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error>> {
        (**self).usage_async()
    }

    fn checksum_async(
        &self,
    ) -> impl Future<Output = Result<Option<IndexMap<String, InterchangeProjectChecksumRaw>>, Self::Error>>
    {
        (**self).checksum_async()
    }

    fn canonical_meta_async(
        &self,
    ) -> impl Future<
        Output = Result<Option<InterchangeProjectMetadataRaw>, CanonicalizationError<Self::Error>>,
    > {
        (**self).canonical_meta_async()
    }

    fn checksum_non_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        (**self).checksum_non_canonical_hex_async()
    }

    fn checksum_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, CanonicalizationError<Self::Error>>> {
        (**self).checksum_canonical_hex_async()
    }

    fn checksum_canonical_variant_async(
        &self,
    ) -> impl Future<Output = Result<ProjectChecksum, Self::Error>> {
        (**self).checksum_canonical_variant_async()
    }
}

// TODO: Eliminate the need for this?
#[derive(Error, Debug)]
pub enum ProjectOrIOError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl<ProjectError> From<FsIoError> for ProjectOrIOError<ProjectError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub trait ProjectMut: ProjectRead {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error>;

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error>;

    fn put_project(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        self.put_info(info, overwrite)?;
        self.put_meta(meta, overwrite)
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), Self::Error>;
}

// ----- Blanket trait impls -----

impl<T: ProjectMut> ProjectMut for &mut T {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        (**self).put_info(info, overwrite)
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        (**self).put_meta(meta, overwrite)
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        (**self).write_source(path, source, overwrite)
    }

    fn put_project(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        (**self).put_project(info, meta, overwrite)
    }
}

/// Intended to wrap an `ProjectRead`, indicating that it should
/// be treated as a (trivial) `ProjectReadAsync`.
#[derive(Debug)]
pub struct AsAsyncProject<T> {
    pub inner: T,
}

#[derive(Debug)]
pub struct AsAsyncReader<T> {
    inner: T,
}

impl<T: Read + Unpin> AsyncRead for AsAsyncReader<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(self.get_mut().inner.read(buf))
    }
}

impl<T: ProjectRead> ProjectReadAsync for AsAsyncProject<T>
where
    for<'a> <T as ProjectRead>::SourceReader<'a>: Unpin,
{
    type Error = <T as ProjectRead>::Error;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.inner.get_project()
    }

    type SourceReader<'a>
        = AsAsyncReader<<T as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(AsAsyncReader {
            inner: self.inner.read_source(path)?,
        })
    }

    async fn sources_async(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        self.inner.sources(ctx)
    }

    // Forward selected `ProjectRead` methods explicitly so that important
    // sync-side overrides on the wrapped `T` are honoured on the async side
    // too. See `ProjectRead::checksum_canonical_hex`'s contract docstring —
    // without that forward, wrapping a `T` that exposes a prefetched digest
    // (e.g. `CachedProject`) would silently fall back to a full download on
    // the async side.
    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        self.inner.get_info()
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        self.inner.get_meta()
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        self.inner.version()
    }

    async fn usage_async(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        self.inner.usage()
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        self.inner.is_definitely_invalid()
    }

    async fn checksum_canonical_hex_async(
        &self,
    ) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        self.inner.checksum_canonical_hex()
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        self.inner.checksum_canonical_variant()
    }
}

/// Wrapper intended to wrap a `ProjectReadAsync`, indicating that it be treated as
/// a `ProjectRead`, using a provided tokio runtime.
#[derive(Debug)]
pub struct AsSyncProjectTokio<T> {
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub inner: T,
}

#[derive(Debug)]
pub struct AsSyncReaderTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
}

impl<T: AsyncRead + Unpin> Read for AsSyncReaderTokio<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use futures::AsyncReadExt as _;
        self.runtime.block_on(async { self.inner.read(buf).await })
    }
}

impl<T: ProjectReadAsync> ProjectRead for AsSyncProjectTokio<T> {
    type Error = <T as ProjectReadAsync>::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.runtime.block_on(self.inner.get_project_async())
    }

    type SourceReader<'a>
        = AsSyncReaderTokio<<T as ProjectReadAsync>::SourceReader<'a>>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        let cloned_runtime = self.runtime.clone();

        self.runtime.block_on(async move {
            Ok(AsSyncReaderTokio {
                runtime: cloned_runtime,
                inner: self.inner.read_source_async(path).await?,
            })
        })
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        self.runtime.block_on(self.inner.sources_async(ctx))
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        self.runtime.block_on(self.inner.get_info_async())
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        self.runtime.block_on(self.inner.get_meta_async())
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        self.runtime.block_on(self.inner.version_async())
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        self.runtime.block_on(self.inner.usage_async())
    }

    fn is_definitely_invalid(&self) -> bool {
        self.runtime
            .block_on(self.inner.is_definitely_invalid_async())
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        self.runtime
            .block_on(self.inner.checksum_canonical_hex_async())
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        self.runtime
            .block_on(self.inner.checksum_canonical_variant_async())
    }
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
