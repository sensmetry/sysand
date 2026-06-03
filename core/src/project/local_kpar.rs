// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    cell::OnceCell,
    fs::{self, File},
    io::{Read, Seek, Write as _},
    num::NonZeroU64,
};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use serde::de::DeserializeOwned;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};
use zip::{ZipArchive, read::ZipFile, result::ZipError};

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        self, KparMeta, ProjectChecksum, ProjectRead, hash_reader,
        utils::{RelativizePathError, ZipArchiveError, relativize_path},
    },
    utils::{lowercase_hex, sha256_lowercase_hex},
};

use super::utils::{FsIoError, ProjectDeserializationError, ToPathBuf, wrapfs};

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

/// Project stored in as a KPar (Zip) archive in the local filesystem.
/// Source file paths are interpreted relative to `root`. Both `.project.json`
/// and `.meta.json` are searched for in `root`. If `root` is not given, it is
/// guessed based on the location of the `.project.json`-file.
///
/// Paths used in the archive are expected to match those used in the metadata
/// manifest (.meta.json)! Sysand *MAY* try to normalize paths in order
/// to match filenames, but no guarantees are made.
///
/// Use `LocalKParProject::new_guess_root` to guess `root` based on the
/// presence of a (presumed unique) `.project.json`.
///
/// The archive is read directly without extracting it.
// TODO: add a way to indicate whether to guess root at construction time
// and use it to indicate that no guessing needed for index kpars
#[derive(Debug)]
pub struct LocalKParProject {
    /// Path used in `Source::LocalKpar` returned by `.sources()`.
    /// If `None` no source will be given.
    /// E.g. if used in lockfile would be the path relative to the lockfile.
    // TODO: Consider removing this and replacing it with some way of
    // relativizing `archive_path` at the call site of .sources().
    pub nominal_path: Option<Utf8UnixPathBuf>,
    /// Path used when locating the project archive internally.
    /// Should be absolute.
    archive_path: Utf8PathBuf,
    expected: Option<KparMeta>,
    /// Optionally specify name of project directory inside archive.
    /// If none, currently always tries to guess before reading
    /// any project files.
    pub root: KparInnerPath,
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
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
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
}

impl From<FsIoError> for LocalKParError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

#[derive(Debug, Error)]
pub enum IntoKparError<ReadError> {
    #[error("missing project information file `.project.json`")]
    MissingInfo,
    #[error("missing project metadata file `.meta.json`")]
    MissingMeta,
    #[error(transparent)]
    ProjectRead(ReadError),
    #[error(transparent)]
    Zip(#[from] ZipArchiveError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("project serialization error: {0}: {1}")]
    Serialize(&'static str, serde_json::Error),
}

impl<ReadError> From<FsIoError> for IntoKparError<ReadError> {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl LocalKParProject {
    pub fn new<P: AsRef<Utf8Path>>(
        path: P,
        root: KparInnerPath,
        nominal: Option<Utf8UnixPathBuf>,
        expected: Option<KparMeta>,
    ) -> Self {
        LocalKParProject {
            nominal_path: nominal,
            root,
            init: OnceCell::new(),
            archive_path: path.to_path_buf(),
            expected,
        }
    }

    pub fn archive_path(&self) -> &Utf8Path {
        &self.archive_path
    }

    fn ensure_initialized(&self) -> Result<&(LocalKParProjectRaw, KparMeta), LocalKParError> {
        // TODO: use `OnceCell::get_or_try_init()` once it's stable;
        // using `get_or_init()` directly requires us to always put the error into an `Arc` to
        // allow returning it repeatedly from functions (`io::Error` is not cloneable)
        match self.init.get() {
            Some(val) => Ok(val),
            None => {
                let (inner, meta) =
                    LocalKParProjectRaw::new_hash(&self.archive_path, self.root.to_owned())?;
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
                            expected: expected.sha256_hex.to_owned(),
                            computed: meta.sha256_hex,
                        });
                    }
                }
                Ok(self.init.get_or_init(|| (inner, meta)))
            }
        }
    }
}

type KParFile<'a> = super::utils::FileWithLifetime<'a>;

// NOTE: Current implementation keeps re-opening the archive file. This appears to
//       be unavoidable with the current design of this trait.
impl ProjectRead for LocalKParProject {
    type Error = LocalKParError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self.ensure_initialized() {
            Ok((inner, _)) => inner.get_project(),
            Err(e) => Err(e),
        }
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        match self.ensure_initialized() {
            Ok((inner, _)) => inner.get_info(),
            Err(e) => Err(e),
        }
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        match self.ensure_initialized() {
            Ok((inner, _)) => inner.get_meta(),
            Err(e) => Err(e),
        }
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
                let kpar_path = if let Some(np) = self.nominal_path.as_ref() {
                    np.as_str().into()
                } else if let Some(w) = &ctx.current_workspace {
                    relativize_path(&inner.archive_path, w.root_path())?
                        .into_string()
                        .into()
                } else if let Some(cp) = &ctx.current_project {
                    relativize_path(&inner.archive_path, cp.root_path())?
                        .into_string()
                        .into()
                } else {
                    panic!(
                        "`LocalKparProject` without `nominal_path` does not have any project sources"
                    );
                };
                Ok(vec![Source::LocalKpar {
                    kpar_path,
                    kpar_size: meta.size_bytes,
                    kpar_digest: meta.sha256_hex.to_owned(),
                }])
            }
            Err(e) => Err(e),
        }
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.ensure_initialized() {
            Ok((_, meta)) => Ok(ProjectChecksum::Kpar(meta.sha256_hex.clone())),
            Err(e) => Err(e),
        }
    }
}

impl LocalKParProjectRaw {
    pub fn new_hash<P: AsRef<Utf8Path>>(
        path: P,
        root: KparInnerPath,
    ) -> Result<(Self, KparMeta), LocalKParError> {
        let path = path.as_ref();
        let size_bytes = match NonZeroU64::new(wrapfs::metadata(path)?.len()) {
            Some(n) => n,
            None => {
                return Err(LocalKParError::EmptyKpar {
                    path: path.as_str().into(),
                });
            }
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
                Some(guess_root(&mut zip)?)
            }
        };

        let project = LocalKParProjectRaw {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.to_path_buf(),
            root,
        };
        Ok((project, meta))
    }

    pub fn new_project_at_root<P: AsRef<Utf8Path>>(path: P) -> Result<Self, Box<FsIoError>> {
        Ok(LocalKParProjectRaw {
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
        let root = Some(guess_root(&mut zip)?);

        Ok(LocalKParProjectRaw {
            tmp_dir: tempdir().map_err(FsIoError::MkTempDir)?,
            archive_path: path.to_path_buf(),
            root,
        })
    }

    pub fn new_temporary() -> Result<Self, Box<FsIoError>> {
        let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;
        Ok(LocalKParProjectRaw {
            archive_path: tmp_dir.path().join("project.kpar"),
            tmp_dir,
            root: None,
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
                Some(guess_root(&mut zip)?)
            }
        };
        Ok(LocalKParProjectRaw {
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

    /// Build a KPAR archive from `from`.
    ///
    /// `extra_files` are added to the archive alongside the project's source
    /// files. Each entry is `(archive_path, content)`; `archive_path` uses `/`
    /// as the separator and is interpreted relative to the archive root.
    pub fn from_project<Pr: ProjectRead, P: AsRef<Utf8Path>>(
        from: &Pr,
        path: P,
        compression: zip::CompressionMethod,
        extra_files: &[(String, String)],
    ) -> Result<Self, IntoKparError<Pr::Error>> {
        let file = wrapfs::File::create(&path)?;
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(compression)
            .system(zip::System::Unix)
            .last_modified_time(zip::DateTime::DEFAULT);

        let (info, meta) = from.get_project().map_err(IntoKparError::ProjectRead)?;
        let info = info.ok_or(IntoKparError::MissingInfo)?;
        let meta = meta.ok_or(IntoKparError::MissingMeta)?;
        let info_content = serde_json::to_string(&info)
            .map_err(|e| IntoKparError::Serialize("failed to serialize project info", e))?;
        let meta_content = serde_json::to_string(&meta)
            .map_err(|e| IntoKparError::Serialize("failed to serialize project metadata", e))?;

        // KerML Clause 10.3: “In addition, the archive shall contain, at its
        // top level, exactly one file named .project.json and exactly one file
        // named .meta.json.”

        zip.start_file(".project.json", options)
            .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".project.json").into(), e))?;
        zip.write(info_content.as_bytes())
            .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

        zip.start_file(".meta.json", options)
            .map_err(|e| ZipArchiveError::Write(Utf8Path::new(".meta.json").into(), e))?;
        zip.write(meta_content.as_bytes())
            .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;

        for source_path in meta.source_paths(true) {
            let mut reader = from
                .read_source(&source_path)
                .map_err(IntoKparError::ProjectRead)?;
            zip.start_file(&source_path, options)
                .map_err(|e| ZipArchiveError::Write(Utf8Path::new(&source_path).into(), e))?;
            std::io::copy(&mut reader, &mut zip)
                .map_err(|e| FsIoError::CopyFile(source_path.into(), path.to_path_buf(), e))?;
        }

        for (archive_path, content) in extra_files {
            zip.start_file(archive_path, options)
                .map_err(|e| ZipArchiveError::Write(Utf8Path::new(archive_path).into(), e))?;
            zip.write_all(content.as_bytes())
                .map_err(|e| FsIoError::WriteFile(path.as_ref().into(), e))?;
        }

        zip.finish()
            .map_err(|e| ZipArchiveError::Finish(path.as_ref().into(), e))?;

        Self::new_project_at_root(&path).map_err(IntoKparError::Io)
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

    fn open_archive(&self) -> Result<ZipArchive<fs::File>, LocalKParError> {
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

    fn get_parsed<T: DeserializeOwned, P: AsRef<Utf8UnixPath>>(
        &self,
        zip: &mut ZipArchive<File>,
        path: P,
    ) -> Result<Option<T>, LocalKParError> {
        match self.get_relative(zip, path) {
            Ok(f) => Ok(Some(serde_json::from_reader(f).map_err(|e| {
                ProjectDeserializationError::new("failed to deserialize `.project.json`", e)
            })?)),
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

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let mut archive = self.open_archive()?;
        let info = self.get_parsed(&mut archive, ".project.json")?;
        let meta = self.get_parsed(&mut archive, ".meta.json")?;
        Ok((info, meta))
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        let mut archive = self.open_archive()?;
        let info = self.get_parsed(&mut archive, ".project.json")?;
        Ok(info)
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        let mut archive = self.open_archive()?;
        let meta = self.get_parsed(&mut archive, ".meta.json")?;
        Ok(meta)
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
        // Ok(KparFile { archive: archive, file: &mut archive.by_index(idx)? })
    }

    /// This always panics. Wrapper is responsible for providing an appropriate source
    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        panic!()
    }

    /// This always panics. Wrapper is responsible for providing the checksum
    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        panic!()
    }
}

/// Guess the directory of the project within the zip archive. Project
/// directory here means any directory that contains `.project.json`.
/// Returned path will be empty if the directory is archive root
fn guess_root(archive: &mut ZipArchive<fs::File>) -> Result<Utf8UnixPathBuf, LocalKParError> {
    let mut maybe_root = None;
    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(ZipArchiveError::FileMeta)?;

        // TODO: do more sanitization here; enclosed_name() does some checks, but
        // it also makes the path OS-native, so Utf8UnixPath won't work with it.
        // To work around this, we check that the sanitized path can be produced,
        // but then use the raw path, as it's always Unix-style per zip spec
        if file.enclosed_name().is_some() {
            let p = Utf8UnixPath::new(file.name());
            if let Some(root) = project_root_from_zip_entry_path(p)? {
                maybe_root = Some(root);
                break;
            }
        }
    }

    if let Some(root) = maybe_root {
        Ok(root)
    } else {
        Err(LocalKParError::NotFound(".project.json".into()))
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
