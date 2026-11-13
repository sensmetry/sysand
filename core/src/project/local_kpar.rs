// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::paths::RelativizePathError;
use std::{
    cell::OnceCell,
    fs::{self, File},
    io::{Read as _, Seek as _},
    num::NonZeroU64,
};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};
use zip::{ZipArchive, read::ZipFile, result::ZipError};

use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::MODEL_TOML_FILE,
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectMetadataPartial,
        InterchangeProjectUsageRaw, PartialManifest,
    },
    project::{
        self, KparMeta, ProjectChecksum, ProjectDefinition, ProjectRead, hash_reader,
        utils::ZipArchiveError,
    },
    utils::{lowercase_hex, sha256_lowercase_hex},
};

use super::utils::{
    FsIoError, ProjectDeserializationError, ToPathBuf as _, read_json_manifest, wrapfs,
};

#[derive(Debug, Clone)]
pub enum KparInnerPath {
    /// Project is at the root of the archive, i.e. accessed directly as
    /// `.project.json` and `.meta.json`
    Root,
    /// Project is at a given path within archive
    Known(Utf8UnixPathBuf),
    /// Project path within archive is unknown and has to be guessed
    Guess,
}

/// Project stored as a KPar (Zip) archive in the local filesystem.
/// Source file paths are resolved relative to and under `root`. Both
/// `.project.json` and `.meta.json` are searched for in `root`.
/// With [`KparInnerPath::Guess`], `root` is the directory of the first
/// `.project.json` found in the archive.
///
/// Paths used in the archive are expected to match those used in the metadata
/// manifest (.meta.json)! Sysand *MAY* try to normalize paths in order
/// to match filenames, but no guarantees are made.
///
/// The archive is read directly without extracting it.
#[derive(Debug)]
pub struct LocalKParProject {
    /// Path used in `Source::LocalKpar` returned by `.sources()`.
    /// If `None` no source will be given.
    /// E.g. if used in lockfile would be the path resolved relative
    /// to the lockfile.
    // TODO: Consider removing this and replacing it with some way of
    // relativizing `archive_path` at the call site of .sources().
    nominal_path: Option<Utf8UnixPathBuf>,
    /// Path used when locating the project archive internally.
    /// Should be absolute.
    archive_path: Utf8PathBuf,
    expected: Option<KparMeta>,
    /// Project directory inside the archive.
    root: KparInnerPath,
    init: OnceCell<(LocalKParProjectRaw, KparMeta)>,
}

/// Assumes that the kpar is already at `archive_path`
#[derive(Debug)]
pub struct LocalKParProjectRaw {
    /// Temporary directory for unpacking files in archive.
    tmp_dir: Utf8TempDir,
    /// Path used when locating the project archive internally.
    /// Should be absolute.
    archive_path: Utf8PathBuf,
    /// Path of project directory inside archive. If `None`, project is
    /// at archive root.
    root: Option<Utf8UnixPathBuf>,
}

#[derive(Error, Debug)]
pub enum LocalKParError {
    #[error(transparent)]
    Zip(#[from] project::utils::ZipArchiveError),
    #[error("path `{0}` not found")]
    NotFound(Box<Utf8Path>),
    /// A KPAR exclusively uses `.project.json`/`.meta.json`.
    #[error(
        "`{archive}` contains `model.toml`;\n\
        {SP:>8} a KPAR carries `.project.json` and `.meta.json`, never a `model.toml`",
        SP = ' '
    )]
    TomlInKpar { archive: Box<Utf8Path> },
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error("kpar at `{path}` has sha256 `{computed}` but the expected digest was `{expected}`")]
    DigestMismatch {
        path: Box<str>,
        expected: String,
        computed: String,
    },
    #[error("kpar at `{path}` has size {actual} bytes but the expected size was {expected} bytes")]
    SizeMismatch {
        path: Box<str>,
        expected: u64,
        actual: u64,
    },
    #[error("kpar at `{path}` is an empty file")]
    EmptyKpar { path: Box<str> },
    #[error("kpar at `{path}` {problem}")]
    Format {
        path: Box<Utf8Path>,
        problem: project::FormatError,
    },
}

impl From<FsIoError> for LocalKParError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl LocalKParProject {
    /// Access the project. No invariants will be checked. If `nominal_path` is provided,
    /// `sources()` will work.
    pub fn new_access(
        path: impl Into<Utf8PathBuf>,
        root: KparInnerPath,
        nominal_path: Option<Utf8UnixPathBuf>,
    ) -> Self {
        Self {
            nominal_path,
            root,
            init: OnceCell::new(),
            archive_path: path.into(),
            expected: None,
        }
    }

    /// Require the archive to have exactly this size and digest; an
    /// index entry's advertised values, verified when the archive is
    /// first opened.
    #[must_use]
    pub fn expecting(mut self, expected: KparMeta) -> Self {
        self.expected = Some(expected);
        self
    }

    /// Like [`Self::new_access`], but the archive must have the size and
    /// digest `expected`, e.g. those a lockfile records
    pub fn new_with_expected(
        path: impl Into<Utf8PathBuf>,
        root: KparInnerPath,
        nominal_path: Option<Utf8UnixPathBuf>,
        expected: KparMeta,
    ) -> Self {
        Self {
            expected: Some(expected),
            ..Self::new_access(path, root, nominal_path)
        }
    }

    pub fn archive_path(&self) -> &Utf8Path {
        &self.archive_path
    }

    fn ensure_initialized(&self) -> Result<&(LocalKParProjectRaw, KparMeta), LocalKParError> {
        // TODO: use `OnceCell::get_or_try_init()` once it's stable;
        // using `get_or_init()` directly requires us to always put the error into an `Arc` to
        // allow returning it repeatedly from functions (`io::Error` is not cloneable)
        if let Some(val) = self.init.get() {
            Ok(val)
        } else {
            let (inner, meta) =
                LocalKParProjectRaw::new_hash(&self.archive_path, self.root.clone())?;
            // TODO: move these sorts of checks out of this type
            if let Some(expected) = &self.expected {
                if meta.size_bytes != expected.size_bytes {
                    return Err(LocalKParError::SizeMismatch {
                        path: self.archive_path.as_str().into(),
                        expected: expected.size_bytes.get(),
                        actual: meta.size_bytes.get(),
                    });
                } else if meta.sha256_hex != expected.sha256_hex {
                    return Err(LocalKParError::DigestMismatch {
                        path: self.archive_path.as_str().into(),
                        expected: expected.sha256_hex.clone(),
                        computed: meta.sha256_hex,
                    });
                }
            }
            Ok(self.init.get_or_init(|| (inner, meta)))
        }
    }
}

type KParFile<'a> = super::utils::FileWithLifetime<'a>;

// NOTE: Current implementation keeps re-opening the archive file. This appears to
//       be unavoidable with the current design of this trait.
impl ProjectRead for LocalKParProject {
    type Error = LocalKParError;

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        match self.ensure_initialized() {
            Ok((inner, _)) => inner.get_project(),
            Err(e) => Err(e),
        }
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        self.ensure_initialized()?.0.version()
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        self.ensure_initialized()?.0.usage()
    }

    type SourceReader<'a>
        = KParFile<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.ensure_initialized() {
            Ok((inner, _)) => inner.read_source(path),
            Err(e) => Err(e),
        }
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self.ensure_initialized() {
            Ok((inner, meta)) => {
                let kpar_path = ctx
                    .lockfile_path(self.nominal_path.as_ref(), &inner.archive_path)?
                    .into_string()
                    .into();
                Ok(vec![Source::LocalKpar {
                    kpar_path,
                    kpar_size: meta.size_bytes,
                    kpar_digest: meta.sha256_hex.clone(),
                }])
            }
            Err(e) => Err(e),
        }
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A path names one archive.
        false
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.ensure_initialized() {
            Ok((_, meta)) => Ok(ProjectChecksum::Kpar(meta.sha256_hex.clone())),
            Err(e) => Err(e),
        }
    }

    fn project_root(&self) -> Option<&Utf8Path> {
        // TODO: is this appropriate? KPAR is not really meant
        // to be tied to a specific folder
        None
    }
}

impl LocalKParProjectRaw {
    pub fn new_hash<P: AsRef<Utf8Path>>(
        path: P,
        root: KparInnerPath,
    ) -> Result<(Self, KparMeta), LocalKParError> {
        let path = path.as_ref();
        let Some(size_bytes) = NonZeroU64::new(wrapfs::metadata(path)?.len()) else {
            return Err(LocalKParError::EmptyKpar {
                path: path.as_str().into(),
            });
        };
        let mut archive = wrapfs::File::open(path)?;
        let sha256_hex = match hash_reader(&mut archive) {
            Ok(digest) => lowercase_hex(digest),
            Err(e) => {
                return Err(LocalKParError::Io(
                    FsIoError::ReadFile(path.to_owned(), e).into(),
                ));
            }
        };

        let meta = KparMeta {
            size_bytes,
            sha256_hex,
        };
        let root = match root {
            KparInnerPath::Root => None,
            KparInnerPath::Known(path) => Some(path),
            KparInnerPath::Guess => {
                archive
                    .rewind()
                    .map_err(|e| FsIoError::Seek(path.to_owned(), 0, e))?;
                let mut zip = zip::ZipArchive::new(archive)
                    .map_err(|e| ZipArchiveError::ReadArchive(path.into(), e))?;
                Some(guess_root(&mut zip, path)?)
            }
        };

        let project = Self {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.to_path_buf(),
            root,
        };
        Ok((project, meta))
    }

    pub fn new_project_at_root<P: AsRef<Utf8Path>>(path: P) -> Result<Self, Box<FsIoError>> {
        Ok(Self {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.to_path_buf(),
            root: None,
        })
    }

    pub fn new_guess_root<P: AsRef<Utf8Path>>(path: P) -> Result<Self, LocalKParError> {
        let path = path.as_ref();
        let archive = wrapfs::File::open(path)?;

        let mut zip = zip::ZipArchive::new(archive)
            .map_err(|e| ZipArchiveError::ReadArchive(path.into(), e))?;
        let root = Some(guess_root(&mut zip, path)?);

        Ok(Self {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.to_path_buf(),
            root,
        })
    }

    pub fn new_tempdir<P: AsRef<Utf8Path>>(
        tmp_dir: Utf8TempDir,
        path: P,
        root: KparInnerPath,
    ) -> Result<Self, LocalKParError> {
        let path = path.as_ref();
        let root = match root {
            KparInnerPath::Root => None,
            KparInnerPath::Known(path) => Some(path),
            KparInnerPath::Guess => {
                let archive = wrapfs::File::open(path)?;

                let mut zip = zip::ZipArchive::new(archive)
                    .map_err(|e| ZipArchiveError::ReadArchive(path.into(), e))?;
                Some(guess_root(&mut zip, path)?)
            }
        };
        Ok(Self {
            archive_path: path.to_owned(),
            tmp_dir,
            root,
        })
    }

    pub fn archive_path(&self) -> &Utf8Path {
        &self.archive_path
    }

    /// Returns project root in archive. If `None`, project is at the
    /// root of the archive
    pub fn project_root_in_archive(&self) -> Option<&Utf8UnixPath> {
        // TODO: maybe it'd be worth enforcing that Some(p) => p is not empty?
        //       Would simplify a bunch of places which currently must check
        //       both
        self.root.as_deref()
    }

    pub fn file_size(&self) -> Result<u64, LocalKParError> {
        Ok(wrapfs::metadata(&self.archive_path)?.len())
    }

    pub fn digest_sha256(&self) -> Result<String, LocalKParError> {
        let mut file = self.open_archive_file()?;
        let mut buf = [0; 1024];
        let mut hasher = Sha256::new();
        loop {
            let count = file
                .read(&mut buf)
                .map_err(|e| FsIoError::ReadFile(self.archive_path.clone(), e))?;
            if count > 0 {
                hasher.update(&buf[..count]);
            } else {
                break;
            }
        }
        Ok(lowercase_hex(hasher.finalize()))
    }

    fn open_archive_file(&self) -> Result<fs::File, LocalKParError> {
        Ok(wrapfs::File::open(&self.archive_path)?)
    }

    pub(crate) fn open_archive(&self) -> Result<ZipArchive<fs::File>, LocalKParError> {
        Ok(zip::ZipArchive::new(self.open_archive_file()?)
            .map_err(|e| ZipArchiveError::ReadArchive(self.archive_path.as_path().into(), e))?)
    }

    /// `path` must be relative and use Unix separators
    fn get_relative<'a, P: AsRef<Utf8UnixPath>>(
        &self,
        zip: &'a mut ZipArchive<File>,
        path: P,
    ) -> Result<ZipFile<'a, File>, (Utf8UnixPathBuf, ZipError)> {
        let path_in_zip = match self.root.as_ref() {
            Some(p) => p.join(path.as_ref()),
            None => path.as_ref().into(),
        };
        match zip.by_path(path_in_zip.as_str()) {
            Ok(f) => Ok(f),
            Err(e) => Err((path_in_zip, e)),
        }
    }

    /// A KPAR never carries a `model.toml`, so one beside the archive's
    /// project root is an error. A `model.toml` deeper in is one of the
    /// project's own files.
    fn ensure_no_model_toml(&self, zip: &mut ZipArchive<File>) -> Result<(), LocalKParError> {
        match self.get_relative(zip, MODEL_TOML_FILE) {
            Ok(_) => Err(LocalKParError::TomlInKpar {
                archive: self.archive_path.as_path().into(),
            }),
            Err((_, ZipError::FileNotFound)) => Ok(()),
            Err((path, err)) => Err(LocalKParError::Zip(ZipArchiveError::NamedFileMeta(
                path.into_string().into(),
                err,
            ))),
        }
    }

    /// Reads the manifest document `T` out of the archive, `None` when the
    /// archive has none.
    fn get_parsed<T: PartialManifest>(
        &self,
        zip: &mut ZipArchive<File>,
        metamodel: Option<&str>,
    ) -> Result<Option<T::Validated>, LocalKParError> {
        match self.get_relative(zip, T::FILE) {
            Ok(f) => Ok(Some(read_json_manifest::<T>(f, metamodel)?)),
            Err((_, ZipError::FileNotFound)) => Ok(None),
            Err((path, err)) => Err(LocalKParError::Zip(ZipArchiveError::NamedFileMeta(
                path.into_string().into(),
                err,
            ))),
        }
    }
}

// NOTE: Current implementation keeps re-opening the archive file. This appears to
//       be unavoidable with the current design of this trait.
impl ProjectRead for LocalKParProjectRaw {
    type Error = LocalKParError;

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        let mut archive = self.open_archive()?;
        self.ensure_no_model_toml(&mut archive)?;
        // The metadata carries the metamodel, so it is read first.
        let meta = self.get_parsed::<InterchangeProjectMetadataPartial>(&mut archive, None)?;
        let info = self.get_parsed::<InterchangeProjectInfoPartial>(
            &mut archive,
            meta.as_ref().and_then(|meta| meta.metamodel.as_deref()),
        )?;
        ProjectDefinition::from_json(info, meta).map_err(|problem| LocalKParError::Format {
            path: self.archive_path.as_path().into(),
            problem,
        })
    }

    type SourceReader<'a>
        = KParFile<'a>
    where
        Self: 'a;

    // FIXME: this may garble the file if two calls interleave (which can
    // happen via async wrappers).
    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        let tmp_name = sha256_lowercase_hex(path.as_ref());
        let tmp_file_path = self.tmp_dir.path().join(tmp_name);

        if !tmp_file_path.is_file() {
            let mut tmp_file = wrapfs::File::create(&tmp_file_path)?;

            let mut archive = self.open_archive()?;
            let mut zip_file = self
                .get_relative(&mut archive, path)
                .map_err(|(p, e)| ZipArchiveError::NamedFileMeta(p.into_string().into(), e))?;

            std::io::copy(&mut zip_file, &mut tmp_file)
                .map_err(|e| FsIoError::WriteFile(tmp_file_path.clone(), e))?;
        }

        Ok(super::utils::FileWithLifetime::new(wrapfs::File::open(
            tmp_file_path,
        )?))

        // TODO: Solve this with a ZipFile-handle instead
    }

    /// This always panics. Wrapper is responsible for providing an appropriate source
    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        panic!()
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A single archive, however the wrapper ends up naming it.
        false
    }

    /// This always panics. Wrapper is responsible for providing the checksum
    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        panic!()
    }

    /// This always panics. Wrapper is responsible for deciding whether to
    /// provide a path
    fn project_root(&self) -> Option<&Utf8Path> {
        panic!()
    }
}

/// Guess the directory of the project within `archive`, the zip file at
/// `path`. Project directory here means any directory that contains
/// `.project.json`. The returned path is empty if the directory is the
/// archive root.
///
/// A `model.toml` seen on the way does not stop the scan, and is reported
/// only if no `.project.json` is found at all, so that a `model.toml` that
/// is one of the project's own files cannot win on entry order.
fn guess_root(
    archive: &mut ZipArchive<fs::File>,
    path: &Utf8Path,
) -> Result<Utf8UnixPathBuf, LocalKParError> {
    let mut maybe_root = None;
    let mut model_toml = false;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(ZipArchiveError::FileMeta)?;

        // TODO: do more sanitization here; enclosed_name() does some checks, but
        // it also makes the path OS-native, so Utf8UnixPath won't work with it.
        // To work around this, we check that the sanitized path can be produced,
        // but then use the raw path, as it's always Unix-style per zip spec
        if file.enclosed_name().is_some() {
            let name = file.name().map_err(ZipArchiveError::FileMeta)?;
            let p = Utf8UnixPath::new(name.as_ref());
            if let Some(root) = project_root_from_zip_entry_path(p)? {
                maybe_root = Some(root);
                break;
            }
            model_toml |= p.file_name() == Some(MODEL_TOML_FILE);
        }
    }

    match (maybe_root, model_toml) {
        (Some(root), _) => Ok(root),
        (None, true) => Err(LocalKParError::TomlInKpar {
            archive: path.into(),
        }),
        (None, false) => Err(LocalKParError::NotFound(".project.json".into())),
    }
}

fn project_root_from_zip_entry_path(
    p: &Utf8UnixPath,
) -> Result<Option<Utf8UnixPathBuf>, ZipArchiveError> {
    if p.file_name() == Some(".project.json") {
        Ok(Some(
            p.parent()
                .ok_or_else(|| ZipArchiveError::InvalidPath(Utf8Path::new(p.as_str()).into()))?
                .to_path_buf(),
        ))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "./local_kpar_tests.rs"]
mod tests;
