// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::model::{
    InterchangeProjectChecksum, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
    InterchangeProjectUsageRaw, ProjectHash, project_hash_raw,
};
use futures::io::{AsyncBufReadExt as _, AsyncRead};
use indexmap::IndexMap;
use sha2::{Digest, Sha256};
use std::{
    fmt::Debug,
    io::{self, BufRead as _, BufReader, Read},
    marker::Unpin,
    sync::Arc,
};
use thiserror::Error;
use typed_path::Utf8UnixPath;
use utils::FsIoError;

// Implementations
pub mod editable;
#[cfg(all(feature = "filesystem", feature = "networking"))]
pub mod gix_git_download;
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

pub mod utils;

fn hash_reader<R: Read>(reader: &mut R) -> Result<ProjectHash, io::Error> {
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
pub enum CanonicalisationError<ReadError> {
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error("failed to read from file\n  '{0}':\n  {1}")]
    FileRead(Box<str>, io::Error),
}

#[derive(Debug, Error)]
pub enum IntoProjectError<ReadError, W: ProjectMut> {
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error(transparent)]
    ProjectWrite(W::Error),
    #[error("missing project information file '.project.json'")]
    MissingInfo,
    #[error("missing project metadata file '.meta.json'")]
    MissingMeta,
}

/// Anything implementing `ProjectRead` can be treated as a method for accessing (one
/// particular) interchange project.
pub trait ProjectRead {
    // Mandatory

    type Error: std::error::Error + Debug;

    /// Fetch project information and metadata (if they exist).
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
    /// May be empty if no valid sources are known.
    fn sources(&self) -> Vec<crate::lock::Source>;

    // Optional and helpers

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(self.get_project()?.0)
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(self.get_project()?.1)
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
    ) -> Result<Option<IndexMap<String, InterchangeProjectChecksum>>, Self::Error> {
        Ok(self.get_meta()?.and_then(|meta| meta.checksum))
    }

    /// Produces canonicalised project metadata, replacing all source file hashes by SHA256.
    fn canonical_meta(
        &self,
    ) -> Result<Option<InterchangeProjectMetadataRaw>, CanonicalisationError<Self::Error>> {
        let Some(mut meta) = self
            .get_meta()
            .map_err(CanonicalisationError::ProjectRead)?
        else {
            return Ok(None);
        };

        for (path, checksum) in meta
            .checksum
            .as_mut()
            .into_iter()
            .flat_map(|index| index.iter_mut())
        {
            if checksum.algorithm != "SHA256" {
                checksum.algorithm = "SHA256".to_string();

                let mut src = self
                    .read_source(path)
                    .map_err(CanonicalisationError::ProjectRead)?;
                checksum.value = format!(
                    "{:x}",
                    hash_reader(&mut src)
                        .map_err(|e| CanonicalisationError::FileRead(path.as_str().into(), e))?
                );
            } else {
                checksum.value = checksum.value.to_lowercase();
            }
        }

        Ok(Some(meta))
    }

    /// Produces a project hash based on project information and the *non-canonicalised* metadata.
    fn checksum_noncanonical_hex(&self) -> Result<Option<String>, Self::Error> {
        Ok(self
            .get_project()
            .map(|(info, meta)| info.zip(meta))?
            .map(|(info, meta)| format!("{:x}", project_hash_raw(&info, &meta))))
    }

    /// Produces a project hash based on project information and the *canonicalised* metadata.
    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalisationError<Self::Error>> {
        let info = self
            .get_info()
            .map_err(CanonicalisationError::ProjectRead)?;
        let meta = self.canonical_meta()?;

        Ok(info
            .zip(meta)
            .map(|(info, meta)| format!("{:x}", project_hash_raw(&info, &meta))))
    }

    // TODO: Make this return an associated type instead?
    /// Treat this `ProjectRead` as a (trivial) `ProjectReadAsync`
    fn to_async(self) -> AsAsyncProject<Self>
    where
        Self: Sized,
    {
        AsAsyncProject { inner: self }
    }
}

pub trait ProjectReadAsync {
    // Mandatory

    type Error: std::error::Error + Debug;

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
    fn sources_async(&self) -> impl Future<Output = Vec<crate::lock::Source>>;

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
    ) -> impl Future<Output = Result<Option<IndexMap<String, InterchangeProjectChecksum>>, Self::Error>>
    {
        async { Ok(self.get_meta_async().await?.and_then(|meta| meta.checksum)) }
    }

    /// Produces canonicalised project metadata, replacing all source file hashes by SHA256.
    fn canonical_meta_async(
        &self,
    ) -> impl Future<
        Output = Result<Option<InterchangeProjectMetadataRaw>, CanonicalisationError<Self::Error>>,
    > {
        async move {
            let Some(mut meta) = self
                .get_meta_async()
                .await
                .map_err(CanonicalisationError::ProjectRead)?
            else {
                return Ok(None);
            };

            if let Some(mut checksums) = meta.checksum {
                let future_checksums = checksums.drain(..).map(|(path, mut checksum)| async move {
                    if checksum.algorithm != "SHA256" {
                        checksum.algorithm = "SHA256".to_string();

                        let mut src = self
                            .read_source_async(&path)
                            .await
                            .map_err(CanonicalisationError::ProjectRead)?;
                        checksum.value = format!(
                            "{:x}",
                            hash_reader_async(&mut src).await.map_err(|e| {
                                CanonicalisationError::FileRead(path.to_string().into(), e)
                            })?
                        );
                    } else {
                        checksum.value = checksum.value.to_lowercase();
                    }

                    Ok((path, checksum))
                });

                let collected_checksums: Result<Vec<(String, InterchangeProjectChecksum)>, _> =
                    futures::future::join_all(future_checksums.into_iter())
                        .await
                        .into_iter()
                        .collect();

                meta.checksum = Some(indexmap::IndexMap::from_iter(collected_checksums?));
            }

            Ok(Some(meta))
        }
    }

    /// Produces a project hash based on project information and the *non-canonicalised* metadata.
    fn checksum_noncanonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async {
            Ok(self
                .get_project_async()
                .await
                .map(|(info, meta)| info.zip(meta))?
                .map(|(info, meta)| format!("{:x}", project_hash_raw(&info, &meta))))
        }
    }

    /// Produces a project hash based on project information and the *canonicalised* metadata.
    fn checksum_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, CanonicalisationError<Self::Error>>> {
        async {
            let info = self
                .get_info_async()
                .await
                .map_err(CanonicalisationError::ProjectRead)?;
            let meta = self.canonical_meta_async().await?;

            Ok(info
                .zip(meta)
                .map(|(info, meta)| format!("{:x}", project_hash_raw(&info, &meta))))
        }
    }

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

#[derive(Debug)]
pub struct IndexMergeOutcome {
    pub new: Vec<String>,
    pub existing: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct SourceExclusionOutcome {
    pub removed_checksum: Option<InterchangeProjectChecksum>,
    pub removed_symbols: Vec<String>,
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

    // Utilities

    fn include_source<P: AsRef<Utf8UnixPath>>(
        &mut self,
        path: P,
        compute_checksum: bool,
        overwrite: bool,
    ) -> Result<(), ProjectOrIOError<Self::Error>> {
        let mut meta = self
            .get_meta()
            .map_err(ProjectOrIOError::Project)?
            .unwrap_or_else(InterchangeProjectMetadataRaw::generate_blank);

        {
            let mut reader = self.read_source(&path).map_err(ProjectOrIOError::Project)?;

            if compute_checksum {
                let sha256_checksum = hash_reader(&mut reader)
                    .map_err(|e| FsIoError::ReadFile(path.as_ref().as_str().into(), e))?;

                meta.add_checksum(&path, "SHA256", format!("{:x}", sha256_checksum), overwrite);
            } else {
                meta.add_checksum(&path, "NONE", "", overwrite);
            }
        }

        self.put_meta(&meta, true)
            .map_err(ProjectOrIOError::Project)
    }

    fn exclude_source<P: AsRef<Utf8UnixPath>>(
        &mut self,
        path: P,
    ) -> Result<SourceExclusionOutcome, ProjectOrIOError<Self::Error>> {
        let mut meta = self
            .get_meta()
            .map_err(ProjectOrIOError::Project)?
            .unwrap_or_else(InterchangeProjectMetadataRaw::generate_blank);

        let removed_checksum = meta.remove_checksum(&path);
        let removed_symbols = meta.remove_index(&path);

        self.put_meta(&meta, true)
            .map_err(ProjectOrIOError::Project)?;

        Ok(SourceExclusionOutcome {
            removed_checksum,
            removed_symbols,
        })
    }

    fn merge_index<S: AsRef<str>, P: AsRef<str>, I: Iterator<Item = (S, P)>>(
        &mut self,
        symbols: I,
        overwrite: bool,
    ) -> Result<IndexMergeOutcome, ProjectOrIOError<Self::Error>> {
        let mut meta = self
            .get_meta()
            .map_err(ProjectOrIOError::Project)?
            .unwrap_or_else(InterchangeProjectMetadataRaw::generate_blank);

        let mut new = vec![];
        let mut existing = vec![];

        for (symbol, path) in symbols {
            let this_symbol = symbol.as_ref().to_string();
            match meta.index.entry(this_symbol.clone()) {
                indexmap::map::Entry::Occupied(mut occupied_entry) => {
                    let current = if overwrite {
                        occupied_entry.insert(path.as_ref().to_string())
                    } else {
                        occupied_entry.get().clone()
                    };

                    existing.push((this_symbol, current));
                }
                indexmap::map::Entry::Vacant(vacant_entry) => {
                    vacant_entry.insert(path.as_ref().to_string());
                    new.push(this_symbol);
                }
            }
        }

        self.put_meta(&meta, true)
            .map_err(ProjectOrIOError::Project)?;

        Ok(IndexMergeOutcome { new, existing })
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

    async fn sources_async(&self) -> Vec<crate::lock::Source> {
        self.inner.sources()
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

    fn sources(&self) -> Vec<crate::lock::Source> {
        self.runtime.block_on(self.inner.sources_async())
    }

    fn is_definitely_invalid(&self) -> bool {
        self.runtime
            .block_on(self.inner.is_definitely_invalid_async())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use indexmap::IndexMap;
    use typed_path::Utf8UnixPath;

    use crate::{
        model::{
            InterchangeProjectChecksum, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        },
        project::{ProjectRead, hash_reader, memory::InMemoryProject},
    };

    #[test]
    fn test_sanity_check_hasher() -> Result<(), Box<dyn std::error::Error>> {
        let input = "FooBarBaz";

        // echo -n "FooBarBaz" | sha256sum
        assert_eq!(
            format!("{:x}", hash_reader(&mut std::io::Cursor::new(input))?),
            "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed"
        );

        Ok(())
    }

    #[test]
    fn test_canonicalisation_no_checksums() -> Result<(), Box<dyn std::error::Error>> {
        let project = InMemoryProject {
            info: Some(InterchangeProjectInfoRaw {
                name: "test_canonicalisation".to_string(),
                description: None,
                version: "1.2.3".to_string(),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: vec![],
            }),
            meta: Some(InterchangeProjectMetadataRaw {
                index: IndexMap::default(),
                created: "123".to_string(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: Some(IndexMap::from([(
                    "MyFile.txt".to_string(),
                    InterchangeProjectChecksum {
                        algorithm: "None".to_string(),
                        value: "".to_string(),
                    },
                )])),
            }),
            files: HashMap::from([(
                Utf8UnixPath::new("MyFile.txt").to_path_buf(),
                "FooBarBaz".to_string(),
            )]),
            nominal_sources: vec![],
        };

        let Some(canonical_info) = project.canonical_meta()? else {
            panic!()
        };

        let Some(checksums) = canonical_info.checksum else {
            panic!()
        };

        assert_eq!(checksums.len(), 1);
        assert_eq!(
            checksums.get("MyFile.txt"),
            Some(&InterchangeProjectChecksum {
                value: "4da8b89a905445e96dd0ab6c9be9a72c8b0ffc686a57a3cc6808a8952a3560ed"
                    .to_string(),
                algorithm: "SHA256".to_string()
            })
        );

        Ok(())
    }
}
