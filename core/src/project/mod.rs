// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8Path;
use digest::{array::Array, typenum};
use futures::io::{AsyncBufReadExt as _, AsyncRead};
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};
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
    cancel::{Cancellable, CancellationToken, bridge},
    context::ProjectContext,
    env::utils::ErrorBound,
    lock::Source,
    manifest::{edit::DocumentMut, project::ResolvedProject},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsage, InterchangeProjectUsageRaw, InterchangeProjectValidationError,
        KerMlChecksumAlg, ProjectHash, project_hash_hex,
    },
    utils::lowercase_hex,
};

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
// TODO: reintroduce `reqwest_kpar_ranged`
#[cfg(feature = "networking")]
pub mod reqwest_src;

pub mod cached;
pub mod reference;

pub mod utils;

#[derive(Debug, Clone)]
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
    #[error("failed to read from file\n  `{0}`")]
    FileRead(Box<str>, #[source] io::Error),
}

impl<E: ErrorBound> CanonicalizationError<E> {
    /// Map the inner `ProjectRead` error type, leaving `FileRead` untouched.
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

/// Why a project's manifest could not be had.
#[derive(Debug, Error)]
pub enum ManifestError<ReadError> {
    #[error(transparent)]
    Project(ReadError),
    /// The project is not written as a `model.toml`. Converting a json
    /// project's documents can fail, so it is left to the command that
    /// reports those failures.
    #[error("this project has no `model.toml` manifest")]
    NoTomlManifest,
}

/// Why a manifest could not be stored.
///
/// The same shape as [`ManifestError`]: a default trait body cannot produce
/// a `Self::Error`, which belongs to the implementation.
#[derive(Error, Debug)]
pub enum ManifestWriteError<WriteError> {
    #[error(transparent)]
    Project(WriteError),
    /// The project has nowhere to put one (a KPAR, a remote source project,
    /// an index entry).
    #[error("this project cannot store a `model.toml` manifest")]
    Unsupported,
}

impl<E: ErrorBound> ManifestWriteError<E> {
    /// Map the inner write error, leaving the rest alone. Used by the
    /// `ProjectMut` derive to rewrap a variant's error.
    pub fn map_project_write<F, E2>(self, f: F) -> ManifestWriteError<E2>
    where
        F: FnOnce(E) -> E2,
        E2: ErrorBound,
    {
        match self {
            Self::Project(e) => ManifestWriteError::Project(f(e)),
            Self::Unsupported => ManifestWriteError::Unsupported,
        }
    }
}

impl<E: ErrorBound> ManifestError<E> {
    /// Map the inner read error, leaving the rest alone. Used by the
    /// `ProjectRead` derive to rewrap a variant's error.
    pub fn map_project_read<F, E2>(self, f: F) -> ManifestError<E2>
    where
        F: FnOnce(E) -> E2,
        E2: ErrorBound,
    {
        match self {
            Self::Project(e) => ManifestError::Project(f(e)),
            Self::NoTomlManifest => ManifestError::NoTomlManifest,
        }
    }
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
            Self::Project(c) => write!(f, "kpar:{c}"),
            Self::Kpar(c) => write!(f, "src:{c}"),
        }
    }
}

/// Which manifest a directory holds, and so which kind of project it is.
///
/// A directory with only json config or only toml config is a project of
/// that kind; one with both is an error, as it is unclear whether it is
/// a non-zipped KPAR or a development project.
///
/// The json side is *either* of the two json manifests: a directory
/// holding only a `.meta.json` is a broken json project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectFormat {
    Json,
    Toml,
    Both,
    Neither,
}

impl ProjectFormat {
    /// The format of a project that has a `model.toml` (`toml`) and a
    /// json manifest (`json`), or neither.
    pub fn from_presence(toml: bool, json: bool) -> Self {
        match (toml, json) {
            (true, true) => Self::Both,
            (true, false) => Self::Toml,
            (false, true) => Self::Json,
            (false, false) => Self::Neither,
        }
    }
}

/// A project's manifest, as written: a `model.toml`, or the
/// `.project.json` and `.meta.json` pair.
///
/// A json project has both documents; one missing either is an error of
/// the project that read it.
#[derive(Clone, Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "one value per read, matched by every caller; boxing the json documents \
              would only add indirection there"
)]
pub enum ProjectDefinition {
    /// The `model.toml`, resolved against the workspace the project is a
    /// member of.
    Toml(Box<ResolvedProject>),
    Json {
        info: InterchangeProjectInfoRaw,
        meta: InterchangeProjectMetadataRaw,
    },
}

impl ProjectDefinition {
    pub fn name(&self) -> &str {
        match self {
            Self::Toml(project) => project.name.as_str(),
            Self::Json { info, meta: _ } => &info.name,
        }
    }

    pub fn publisher(&self) -> Option<&str> {
        match self {
            Self::Toml(project) => Some(project.publisher.as_str()),
            Self::Json { info, meta: _ } => info.publisher.as_deref(),
        }
    }

    pub fn version(&self) -> String {
        match self {
            Self::Toml(project) => project.version.to_string(),
            Self::Json { info, meta: _ } => info.version.clone(),
        }
    }

    pub fn license(&self) -> Option<String> {
        match self {
            Self::Toml(project) => project.license.as_ref().map(ToString::to_string),
            Self::Json { info, meta: _ } => info.license.clone(),
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            Self::Toml(project) => project.description.as_deref(),
            Self::Json { info, meta: _ } => info.description.as_deref(),
        }
    }

    pub fn website(&self) -> Option<&str> {
        match self {
            Self::Toml(project) => project.urls.website.as_ref().map(fluent_uri::Iri::as_str),
            Self::Json { info, meta: _ } => info.website.as_deref(),
        }
    }

    pub fn maintainers(&self) -> Vec<String> {
        match self {
            Self::Toml(project) => project
                .maintainers
                .iter()
                .map(ToString::to_string)
                .collect(),
            Self::Json { info, meta: _ } => info.maintainer.clone(),
        }
    }

    pub fn topics(&self) -> &[String] {
        match self {
            Self::Toml(project) => &project.topics,
            Self::Json { info, meta: _ } => &info.topic,
        }
    }

    /// The usages as a `.project.json` carries them; a `model.toml` usage
    /// is mapped by [`crate::manifest::kpar_view::usage`].
    pub fn usages(&self) -> Vec<InterchangeProjectUsageRaw> {
        match self {
            Self::Toml(project) => project
                .usages
                .iter()
                .map(|located| crate::manifest::kpar_view::usage(&located.usage))
                .collect(),
            Self::Json { info, meta: _ } => info.usage.clone(),
        }
    }

    pub fn metamodel(&self) -> Option<String> {
        match self {
            Self::Toml(project) => Some(project.metamodel.to_string()),
            Self::Json { info: _, meta } => meta.metamodel.clone(),
        }
    }

    /// The usages, validated. A json project's whole `.project.json` is
    /// validated; a `model.toml` was when it was read.
    pub fn validated_usages(
        &self,
    ) -> Result<Vec<InterchangeProjectUsage>, InterchangeProjectValidationError> {
        match self {
            Self::Toml(_) => self
                .usages()
                .iter()
                .map(InterchangeProjectUsageRaw::validate)
                .collect(),
            Self::Json { info, meta: _ } => Ok(info.validate()?.usage),
        }
    }

    pub fn format(&self) -> ProjectFormat {
        match self {
            Self::Toml(_) => ProjectFormat::Toml,
            Self::Json { info: _, meta: _ } => ProjectFormat::Json,
        }
    }

    /// The json documents, or `None` for a `model.toml` project.
    pub fn json(&self) -> Option<(&InterchangeProjectInfoRaw, &InterchangeProjectMetadataRaw)> {
        match self {
            Self::Toml(_) => None,
            Self::Json { info, meta } => Some((info, meta)),
        }
    }

    /// The json documents, or `None` for a `model.toml` project.
    pub fn into_json(self) -> Option<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
        match self {
            Self::Toml(_) => None,
            Self::Json { info, meta } => Some((info, meta)),
        }
    }

    /// The json documents, or the error naming a `model.toml` project that
    /// has none to report.
    pub fn into_json_documents(
        self,
    ) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), NotJsonProject> {
        let name = self.name().to_owned();
        self.into_json().ok_or(NotJsonProject { name })
    }
}

/// A `model.toml` project where a json project's documents were wanted.
// TODO: report a `model.toml` project's manifest.
#[derive(Debug, Error)]
#[error("`{name}` is a `model.toml` project, which `info` cannot report yet")]
pub struct NotJsonProject {
    name: String,
}

/// Why a `model.toml` project's symbols could not be extracted.
#[derive(Debug, Error)]
pub enum SymbolIndexError<ReadError> {
    #[error(transparent)]
    Project(ReadError),
    #[error(transparent)]
    Source(#[from] SourceSymbolsError),
}

/// Why one source file's symbols could not be extracted.
#[derive(Debug, Error)]
pub enum SourceSymbolsError {
    #[error("failed to read source file `{0}`")]
    Read(Box<str>, #[source] io::Error),
    #[error(transparent)]
    Symbols(#[from] crate::symbols::SymbolsError),
}

/// Every top-level symbol a `model.toml` project's sources declare,
/// mapped to the source that declares it, read through `project`.
///
/// A symbol declared in two sources maps to the later one.
pub fn symbol_index<P: ProjectRead + ?Sized>(
    project: &P,
    manifest: &ResolvedProject,
) -> Result<IndexMap<String, String>, SymbolIndexError<P::Error>> {
    crate::manifest::kpar_view::symbol_index(
        &manifest.sources,
        |source| {
            let mut contents = String::new();
            project
                .read_source(source)
                .map_err(SymbolIndexError::Project)?
                .read_to_string(&mut contents)
                .map_err(|e| SourceSymbolsError::Read(source.as_str().into(), e))?;
            Ok(contents)
        },
        |_, _| {},
    )
    .map_err(|e| match e {
        crate::manifest::kpar_view::ViewError::Read(e) => e,
        crate::manifest::kpar_view::ViewError::Symbols(e) => SourceSymbolsError::from(e).into(),
    })
}

/// The top-level symbols `project` exports: what its `.meta.json` lists,
/// or what a `model.toml` project's sources declare.
pub fn exported_symbols<P: ProjectRead + ?Sized>(
    project: &P,
    definition: &ProjectDefinition,
) -> Result<Vec<String>, SymbolIndexError<P::Error>> {
    match definition {
        ProjectDefinition::Json { info: _, meta } => Ok(meta.exported_symbols().cloned().collect()),
        ProjectDefinition::Toml(manifest) => {
            Ok(symbol_index(project, manifest)?.into_keys().collect())
        }
    }
}

#[expect(
    clippy::panic,
    reason = "a project hash is defined only for json projects; hashing a \
              `model.toml` project is a programming error"
)]
fn hashable_json(
    definition: ProjectDefinition,
) -> (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw) {
    match definition {
        ProjectDefinition::Json { info, meta } => (info, meta),
        ProjectDefinition::Toml(project) => panic!(
            "`{}` is a `model.toml` project, which has no project hash",
            project.name.as_str()
        ),
    }
}

/// `meta` with every source file hash replaced by its SHA256.
fn canonicalize_meta<P: ProjectRead + ?Sized>(
    project: &P,
    mut meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw, CanonicalizationError<P::Error>> {
    for (path, checksum) in meta
        .checksum
        .as_mut()
        .into_iter()
        .flat_map(|index| index.iter_mut())
    {
        let sha256: &str = KerMlChecksumAlg::Sha256.into();
        if checksum.algorithm == sha256 {
            checksum.value = checksum.value.to_lowercase();
        } else {
            checksum.algorithm = sha256.to_owned();

            let mut src = project
                .read_source(path)
                .map_err(CanonicalizationError::ProjectRead)?;
            checksum.value = lowercase_hex(
                hash_reader(&mut src)
                    .map_err(|e| CanonicalizationError::FileRead(path.as_str().into(), e))?,
            );
        }
    }
    Ok(meta)
}

/// [`canonicalize_meta`] for an async project, hashing the files concurrently.
async fn canonicalize_meta_async<P: ProjectReadAsync + ?Sized>(
    project: &P,
    mut meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw, CanonicalizationError<P::Error>> {
    if let Some(mut checksums) = meta.checksum {
        let future_checksums = checksums.drain(..).map(|(path, mut checksum)| async move {
            let sha256: &str = KerMlChecksumAlg::Sha256.into();
            if checksum.algorithm == sha256 {
                checksum.value = checksum.value.to_lowercase();
            } else {
                checksum.algorithm = sha256.to_owned();

                let mut src = project
                    .read_source_async(&path)
                    .await
                    .map_err(CanonicalizationError::ProjectRead)?;
                checksum.value = lowercase_hex(
                    hash_reader_async(&mut src)
                        .await
                        .map_err(|e| CanonicalizationError::FileRead(path.clone().into(), e))?,
                );
            }

            Ok((path, checksum))
        });

        let collected_checksums: Result<Vec<(String, InterchangeProjectChecksumRaw)>, _> =
            futures::future::join_all(future_checksums)
                .await
                .into_iter()
                .collect();

        meta.checksum = Some(IndexMap::from_iter(collected_checksums?));
    }
    Ok(meta)
}

/// Anything implementing `ProjectRead` can be treated as a method for accessing (one
/// particular) interchange project.
pub trait ProjectRead {
    // Mandatory

    type Error: ErrorBound;

    /// Read the project's manifest. `Ok(None)` means there is none.
    // TODO: cache project info
    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error>;

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

    /// Whether the source this project came from can offer several versions
    /// of the same project, so that obtaining it involved a version *choice*.
    ///
    /// True for an index, and for anything else that answers by listing what
    /// it holds. False for a source that names one project outright: a path,
    /// a URL, a git checkout, a source override.
    ///
    /// An unconstrained usage defaults its version constraint to `*` when
    /// this is true: like cargo, that selects every release but no
    /// prerelease. When false, defaulting could only reject the one project
    /// on offer.
    fn source_may_offer_multiple_versions(&self) -> bool;

    // Optional and helpers

    /// Returns the local filesystem root path of this project, if available.
    /// It is used, among other things, to resolve relative path usages. Such
    /// usages will fail resolution if `None` is returned here
    fn project_root(&self) -> Option<&Utf8Path>;

    /// Which manifest this project is written in.
    ///
    /// The default is [`ProjectFormat::Json`], right for a project that has
    /// just the two json documents, such as a KPAR or an index entry. A
    /// wrapper must forward this to the project it holds, or it claims json
    /// for a `model.toml` project.
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        Ok(ProjectFormat::Json)
    }

    /// The project's manifest as a document.
    ///
    /// [`ProjectDefinition::Toml`] holds the *values*, with inheritance
    /// markers followed; this returns what the file says, comments and key
    /// order included, which is what an editor needs.
    ///
    /// **The document is validated before it is handed back.** Parsing
    /// TOML is not reading a manifest: a file written for a
    /// `manifest-version` this Sysand does not support parses fine and
    /// must not be edited. So an implementation runs the same read
    /// `get_project()` does, keeps its diagnostics, and returns the
    /// document only when they are empty.
    ///
    /// The default refuses: only a `model.toml` project has one.
    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        Err(ManifestError::NoTomlManifest)
    }

    /// `is_definitely_invalid` will return `true` only if `get_project()` would definitely
    /// produce an error or `Ok(None)`. If it returns `false` nothing definite can be said.
    ///
    /// Implementations may use this to give shortcuts for eliminating potential interchange
    /// projects. *Should* be significantly faster than running `get_project`.
    fn is_definitely_invalid(&self) -> bool {
        false
    }

    /// The project's version, as the solver reads it: what
    /// [`ProjectDefinition::version`] gives, or what a source knows
    /// without reading the manifest (an index entry answers from
    /// `versions.json`).
    ///
    /// A wrapper forwards the wrapped project's answer.
    fn version(&self) -> Result<Option<String>, Self::Error> {
        Ok(self.get_project()?.map(|project| project.version()))
    }

    /// The project's usages, as the solver reads them. See
    /// [`ProjectRead::version`].
    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(self.get_project()?.map(|project| project.usages()))
    }

    /// Produces a project hash based on project information and the *non-canonicalized* metadata.
    ///
    /// # Panics
    ///
    /// On a `model.toml` project, which has no project hash.
    fn checksum_non_canonical_hex(&self) -> Result<Option<String>, Self::Error> {
        Ok(self.get_project()?.map(|definition| {
            let (info, meta) = hashable_json(definition);
            project_hash_hex(&info, &meta)
        }))
    }

    /// Produces a project hash based on project information and the
    /// *canonicalized* metadata, in which every source file hash is SHA256.
    ///
    /// `Ok(None)` means the project has no manifest.
    ///
    /// # Panics
    ///
    /// On a `model.toml` project, which has no project hash.
    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        let Some(definition) = self
            .get_project()
            .map_err(CanonicalizationError::ProjectRead)?
        else {
            return Ok(None);
        };
        let (info, meta) = hashable_json(definition);
        let meta = canonicalize_meta(self, meta)?;
        Ok(Some(project_hash_hex(&info, &meta)))
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
    fn source_may_offer_multiple_versions(&self) -> bool {
        (*self).source_may_offer_multiple_versions()
    }

    type Error = T::Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        (**self).format()
    }

    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        (**self).manifest_document()
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
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

    fn is_definitely_invalid(&self) -> bool {
        (*self).is_definitely_invalid()
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        (*self).version()
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        (*self).usage()
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

    fn project_root(&self) -> Option<&Utf8Path> {
        (*self).project_root()
    }
}

impl<T: ProjectRead> ProjectRead for &mut T {
    fn source_may_offer_multiple_versions(&self) -> bool {
        (**self).source_may_offer_multiple_versions()
    }

    type Error = T::Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        (**self).format()
    }

    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        (**self).manifest_document()
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
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

    fn is_definitely_invalid(&self) -> bool {
        (**self).is_definitely_invalid()
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        (**self).version()
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        (**self).usage()
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

    fn project_root(&self) -> Option<&Utf8Path> {
        (**self).project_root()
    }
}

pub trait ProjectReadAsync {
    // Mandatory

    type Error: ErrorBound;

    /// See [`ProjectRead::get_project`].
    fn get_project_async(
        &self,
    ) -> impl Future<Output = Result<Option<ProjectDefinition>, Self::Error>>;

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
    /// See [`ProjectRead::sources`].
    // TODO: should we require the checksum to be verified (i.e. actual, instead of
    // expected)
    fn sources_async(
        &self,
        ctx: &ProjectContext,
    ) -> impl Future<Output = Result<Vec<Source>, Self::Error>>;

    /// Whether the source this project came from can offer several versions
    /// of the same project, so that obtaining it involved a version *choice*.
    ///
    /// True for an index, and for anything else that answers by listing what
    /// it holds. False for a source that names one project outright: a path,
    /// a URL, a git checkout, a source override.
    ///
    /// An unconstrained usage defaults its version constraint to `*` when
    /// this is true: like cargo, that selects every release but no
    /// prerelease. When false, defaulting could only reject the one project
    /// on offer.
    fn source_may_offer_multiple_versions(&self) -> bool;

    // Optional and helpers

    /// See [`ProjectRead::is_definitely_invalid`].
    fn is_definitely_invalid_async(&self) -> impl Future<Output = bool> {
        async { false }
    }

    /// See [`ProjectRead::version`].
    fn version_async(&self) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async {
            Ok(self
                .get_project_async()
                .await?
                .map(|project| project.version()))
        }
    }

    /// See [`ProjectRead::usage`].
    fn usage_async(
        &self,
    ) -> impl Future<Output = Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error>> {
        async {
            Ok(self
                .get_project_async()
                .await?
                .map(|project| project.usages()))
        }
    }

    /// See [`ProjectRead::checksum_non_canonical_hex`].
    ///
    /// # Panics
    ///
    /// On a `model.toml` project, which has no project hash.
    fn checksum_non_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> {
        async {
            Ok(self.get_project_async().await?.map(|definition| {
                let (info, meta) = hashable_json(definition);
                project_hash_hex(&info, &meta)
            }))
        }
    }

    /// See [`ProjectRead::checksum_canonical_hex`].
    ///
    /// # Panics
    ///
    /// On a `model.toml` project, which has no project hash.
    fn checksum_canonical_hex_async(
        &self,
    ) -> impl Future<Output = Result<Option<String>, CanonicalizationError<Self::Error>>> {
        async {
            let Some(definition) = self
                .get_project_async()
                .await
                .map_err(CanonicalizationError::ProjectRead)?
            else {
                return Ok(None);
            };
            let (info, meta) = hashable_json(definition);
            let meta = canonicalize_meta_async(self, meta).await?;
            Ok(Some(project_hash_hex(&info, &meta)))
        }
    }

    fn checksum_canonical_variant_async(
        &self,
    ) -> impl Future<Output = Result<ProjectChecksum, Self::Error>>;

    // Async doesn't make sense for this
    fn project_root(&self) -> Option<&Utf8Path>;

    /// See [`ProjectRead::format`]. Deciding the format never waits on
    /// anything.
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        Ok(ProjectFormat::Json)
    }

    /// Treat this `ProjectReadAsync` as a `ProjectRead` using the provided tokio runtime.
    fn to_tokio_sync(self, runtime: Arc<tokio::runtime::Runtime>) -> AsSyncProjectTokio<Self>
    where
        Self: Sized,
    {
        AsSyncProjectTokio::new(runtime, self)
    }

    /// [`Self::to_tokio_sync`] observing `token` at every bridge.
    fn to_tokio_sync_with(
        self,
        runtime: Arc<tokio::runtime::Runtime>,
        token: CancellationToken,
    ) -> AsSyncProjectTokio<Self>
    where
        Self: Sized,
    {
        AsSyncProjectTokio::new(runtime, self).with_cancellation(token)
    }
}

impl<T: ProjectReadAsync> ProjectReadAsync for &T {
    fn source_may_offer_multiple_versions(&self) -> bool {
        (*self).source_may_offer_multiple_versions()
    }

    type Error = T::Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        (**self).format()
    }

    fn get_project_async(
        &self,
    ) -> impl Future<Output = Result<Option<ProjectDefinition>, Self::Error>> {
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

    fn project_root(&self) -> Option<&Utf8Path> {
        (**self).project_root()
    }
}

impl<T: ProjectReadAsync> ProjectReadAsync for &mut T {
    fn source_may_offer_multiple_versions(&self) -> bool {
        (**self).source_may_offer_multiple_versions()
    }

    type Error = T::Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        (**self).format()
    }

    fn get_project_async(
        &self,
    ) -> impl Future<Output = Result<Option<ProjectDefinition>, Self::Error>> {
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

    fn project_root(&self) -> Option<&Utf8Path> {
        (**self).project_root()
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

    /// Store a `model.toml` manifest.
    ///
    /// The document is what [`crate::manifest::edit::render_new`] built or
    /// [`crate::manifest::edit::open`] parsed.
    ///
    /// The default refuses.
    ///
    /// `overwrite` governs a manifest that is already there, as for the two
    /// json documents. Whether a project that holds json documents accepts a
    /// manifest beside them is up to the implementation; a directory refuses,
    /// as it would be ambiguous.
    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), ManifestWriteError<Self::Error>> {
        let _ = (document, overwrite);
        Err(ManifestWriteError::Unsupported)
    }
}

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

    // Must be forwarded: the trait default answers `Unsupported`.
    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), ManifestWriteError<Self::Error>> {
        (**self).put_manifest_document(document, overwrite)
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
    fn source_may_offer_multiple_versions(&self) -> bool {
        self.inner.source_may_offer_multiple_versions()
    }

    type Error = <T as ProjectRead>::Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        self.inner.format()
    }

    async fn get_project_async(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
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

    // Forward selected `ProjectRead` methods explicitly so that sync-side
    // overrides on `T` (e.g. a prefetched digest) are honoured on the async
    // side too.
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

    fn project_root(&self) -> Option<&Utf8Path> {
        self.inner.project_root()
    }
}

/// Wrapper intended to wrap a `ProjectReadAsync`, indicating that it be treated as
/// a `ProjectRead`, using a provided tokio runtime. Every bridge observes
/// the cancellation token (`crate::cancel`), so an operation that was
/// cancelled returns [`Cancellable::Cancelled`] at its next call.
#[derive(Debug)]
#[non_exhaustive]
pub struct AsSyncProjectTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
    token: CancellationToken,
}

impl<T> AsSyncProjectTokio<T> {
    /// Bridge `inner` on `runtime`, with no cancellation.
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, inner: T) -> Self {
        Self {
            runtime,
            inner,
            token: CancellationToken::new(),
        }
    }

    /// Observe `token` at every bridge.
    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.token = token;
        self
    }
    /// The bridged project.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// The bridged project, unwrapped.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct AsSyncReaderTokio<T> {
    runtime: Arc<tokio::runtime::Runtime>,
    inner: T,
    token: CancellationToken,
}

impl<T> AsSyncReaderTokio<T> {
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

impl<T: AsyncRead + Unpin> Read for AsSyncReaderTokio<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        use futures::AsyncReadExt as _;
        let Self {
            runtime,
            inner,
            token,
        } = self;
        match runtime.block_on(token.run_until_cancelled(inner.read(buf))) {
            Some(result) => result,
            None => Err(crate::cancel::cancelled_io_error()),
        }
    }
}

impl<T: ProjectReadAsync> ProjectRead for AsSyncProjectTokio<T> {
    fn source_may_offer_multiple_versions(&self) -> bool {
        self.inner.source_may_offer_multiple_versions()
    }

    type Error = Cancellable<<T as ProjectReadAsync>::Error>;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        self.inner.format().map_err(Cancellable::Inner)
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        bridge(&self.runtime, &self.token, self.inner.get_project_async())
    }

    type SourceReader<'a>
        = AsSyncReaderTokio<<T as ProjectReadAsync>::SourceReader<'a>>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        let reader = bridge(
            &self.runtime,
            &self.token,
            self.inner.read_source_async(path),
        )?;
        Ok(AsSyncReaderTokio::new(self.runtime.clone(), reader)
            .with_cancellation(self.token.clone()))
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        bridge(&self.runtime, &self.token, self.inner.sources_async(ctx))
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        bridge(&self.runtime, &self.token, self.inner.version_async())
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        bridge(&self.runtime, &self.token, self.inner.usage_async())
    }

    fn is_definitely_invalid(&self) -> bool {
        // A cancelled probe says nothing; the next `Result` checkpoint
        // reports the cancellation.
        crate::cancel::bridge_value(
            &self.runtime,
            &self.token,
            self.inner.is_definitely_invalid_async(),
        )
        .unwrap_or(false)
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        match crate::cancel::bridge_value(
            &self.runtime,
            &self.token,
            self.inner.checksum_canonical_hex_async(),
        ) {
            Some(Ok(hex)) => Ok(hex),
            Some(Err(CanonicalizationError::ProjectRead(error))) => Err(
                CanonicalizationError::ProjectRead(Cancellable::Inner(error)),
            ),
            Some(Err(CanonicalizationError::FileRead(path, error))) => {
                Err(CanonicalizationError::FileRead(path, error))
            }
            None => Err(CanonicalizationError::ProjectRead(Cancellable::Cancelled(
                crate::cancel::Cancelled,
            ))),
        }
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        bridge(
            &self.runtime,
            &self.token,
            self.inner.checksum_canonical_variant_async(),
        )
    }

    fn project_root(&self) -> Option<&Utf8Path> {
        self.inner.project_root()
    }
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
