// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    borrow::Borrow,
    fmt::Display,
    io::{self, Read},
    ops::Deref,
};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::{
    Iri,
    pct_enc::{EString, encoder::IData},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;
#[cfg(feature = "filesystem")]
use zip::{self, result::ZipError};

use crate::{
    manifest::diagnostics::{Diagnostics, Report, Validated},
    model::{
        IndexName, IndexPublisher, InterchangeProjectUsage, InterchangeProjectUsageRaw,
        ProjectName, ProjectPublisher,
    },
    purl::sysand_purl,
    utils::SP,
};

/// A file that is guaranteed to exist as long as the lifetime.
/// Intended to be used with temporary files that are automatically
/// deleted; in this case, the lifetime `'a` is the lifetime of the
/// temporary file.
pub struct FileWithLifetime<'a> {
    internal: std::fs::File,
    phantom: std::marker::PhantomData<&'a ()>,
}

impl FileWithLifetime<'_> {
    pub fn new(file: std::fs::File) -> Self {
        FileWithLifetime {
            internal: file,
            phantom: std::marker::PhantomData,
        }
    }
}

impl Read for FileWithLifetime<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.internal.read(buf)
    }
}

pub trait ToPathBuf {
    fn to_path_buf(&self) -> Utf8PathBuf;
}

impl<P> ToPathBuf for P
where
    P: AsRef<Utf8Path>,
{
    fn to_path_buf(&self) -> Utf8PathBuf {
        self.as_ref().into()
    }
}

pub trait ToUnixPathBuf {
    fn to_unix_path_buf(&self) -> Utf8UnixPathBuf;
}

impl<P> ToUnixPathBuf for P
where
    P: AsRef<Utf8Path>,
{
    fn to_unix_path_buf(&self) -> Utf8UnixPathBuf {
        let mut unix_path = Utf8UnixPathBuf::new();
        for component in self.as_ref().components() {
            unix_path.push(component.as_str());
        }
        unix_path
    }
}

pub fn serialize_unix_path<S>(path: &Utf8UnixPathBuf, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_str(path.as_str())
}

pub fn deserialize_unix_path<'de, D>(deserializer: D) -> Result<Utf8UnixPathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;
    Ok(Utf8UnixPathBuf::from(string))
}

/// The errors arising from filesystem I/O.
/// The variants defined here include relevant context where possible.
#[derive(Error, Debug)]
pub enum FsIoError {
    #[error("failed to canonicalize path\n  `{0}`")]
    Canonicalize(Utf8PathBuf, #[source] io::Error),
    #[error("failed to make path absolute\n  `{0}`")]
    Absolute(Utf8PathBuf, #[source] io::Error),
    #[error("failed to create directory\n  `{0}`")]
    MkDir(Utf8PathBuf, #[source] io::Error),
    #[error("failed to open file\n  `{0}`")]
    OpenFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to get metadata for\n  `{0}`")]
    Metadata(Utf8PathBuf, #[source] io::Error),
    #[error("failed to create a temporary file")]
    CreateTempFile(#[source] io::Error),
    #[error("failed to create a temporary directory")]
    MkTempDir(#[source] io::Error),
    #[error("failed to write file\n  `{0}`")]
    WriteFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to read directory\n  `{0}`")]
    ReadDir(Utf8PathBuf, #[source] io::Error),
    #[error("failed to read file\n  `{0}`")]
    ReadFile(Utf8PathBuf, #[source] io::Error),
    /// Same as `Self::ReadFile`, but path is unknown when using file handle.
    #[error("failed to read file")]
    ReadFileHandle(#[source] io::Error),
    #[error("failed to move\n  `{0}` to\n  `{1}`")]
    Move(Utf8PathBuf, Utf8PathBuf, #[source] io::Error),
    #[error("failed to create file\n  `{0}`")]
    CreateFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to copy file from\n  `{0}` to\n  `{1}`")]
    CopyFile(Utf8PathBuf, Utf8PathBuf, #[source] io::Error),
    #[error("failed to remove file\n  `{0}`")]
    RmFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to lock file\n  `{0}`")]
    LockFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to remove directory\n  `{0}`")]
    RmDir(Utf8PathBuf, #[source] io::Error),
    #[error("failed to get path to current directory")]
    CurrentDir(#[source] io::Error),
    #[error("failed to get metadata to determine if\n  `{0}` is a regular file")]
    IsFile(Utf8PathBuf, #[source] io::Error),
    #[error("failed to seek file `{0}` to {1}")]
    Seek(Utf8PathBuf, u64, #[source] io::Error),
}

/// Which advisory lock to take on a file: shared for readers, exclusive
/// for writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Shared,
    Exclusive,
}

impl LockMode {
    /// Take this lock on `file` if no conflicting lock is held, without
    /// waiting.
    pub fn try_lock(self, file: &std::fs::File) -> Result<(), std::fs::TryLockError> {
        match self {
            Self::Shared => file.try_lock_shared(),
            Self::Exclusive => file.try_lock(),
        }
    }

    /// Take this lock on `file`, polling every `poll_interval` for up to
    /// `wait` (not at all when `None`). `on_wait` runs once, before the
    /// first sleep.
    pub fn lock_polling(
        self,
        file: &std::fs::File,
        wait: Option<std::time::Duration>,
        poll_interval: std::time::Duration,
        on_wait: impl FnOnce(),
    ) -> Result<(), LockPollError> {
        let deadline = wait.map(|wait| std::time::Instant::now() + wait);
        let mut on_wait = Some(on_wait);
        loop {
            match self.try_lock(file) {
                Ok(()) => return Ok(()),
                Err(std::fs::TryLockError::WouldBlock) => {
                    let Some(deadline) = deadline else {
                        return Err(LockPollError::Blocked);
                    };
                    if std::time::Instant::now() >= deadline {
                        return Err(LockPollError::Blocked);
                    }
                    if let Some(on_wait) = on_wait.take() {
                        on_wait();
                    }
                    std::thread::sleep(poll_interval);
                }
                Err(std::fs::TryLockError::Error(err)) => return Err(LockPollError::Io(err)),
            }
        }
    }
}

/// Why [`LockMode::lock_polling`] gave up.
#[derive(Debug)]
pub enum LockPollError {
    /// Another holder kept the lock for the whole wait.
    Blocked,
    Io(std::io::Error),
}

/// Wrappers for filesystem I/O functions to return `FsIoError`.
/// Copies the `std` interface 1 to 1, except for the error type.
pub mod wrapfs {

    use std::fs;
    use std::io;
    use std::io::ErrorKind;

    use camino::Utf8Path;
    use camino::Utf8PathBuf;

    use super::FsIoError;

    #[expect(non_snake_case)]
    pub mod File {

        use std::fs;

        use camino::Utf8Path;

        use super::FsIoError;

        pub fn open<P: AsRef<Utf8Path>>(path: P) -> Result<fs::File, Box<FsIoError>> {
            fs::File::open(path.as_ref())
                .map_err(|e| Box::new(FsIoError::OpenFile(path.as_ref().into(), e)))
        }

        pub fn create<P: AsRef<Utf8Path>>(path: P) -> Result<fs::File, Box<FsIoError>> {
            fs::File::create(path.as_ref())
                .map_err(|e| Box::new(FsIoError::CreateFile(path.as_ref().into(), e)))
        }

        pub fn create_new<P: AsRef<Utf8Path>>(path: P) -> Result<fs::File, Box<FsIoError>> {
            fs::File::create_new(path.as_ref())
                .map_err(|e| Box::new(FsIoError::CreateFile(path.as_ref().into(), e)))
        }
    }

    pub fn create_dir<P: AsRef<Utf8Path>>(path: P) -> Result<(), Box<FsIoError>> {
        fs::create_dir(path.as_ref())
            .map_err(|e| Box::new(FsIoError::MkDir(path.as_ref().into(), e)))
    }

    pub fn create_dir_all<P: AsRef<Utf8Path>>(path: P) -> Result<(), Box<FsIoError>> {
        fs::create_dir_all(path.as_ref())
            .map_err(|e| Box::new(FsIoError::MkDir(path.as_ref().into(), e)))
    }

    pub fn read_dir<P: AsRef<Utf8Path>>(path: P) -> Result<camino::ReadDirUtf8, Box<FsIoError>> {
        path.as_ref()
            .read_dir_utf8()
            .map_err(|e| Box::new(FsIoError::ReadDir(path.as_ref().into(), e)))
    }

    pub fn remove_dir_all<P: AsRef<Utf8Path>>(path: P) -> Result<(), Box<FsIoError>> {
        fs::remove_dir_all(path.as_ref())
            .map_err(|e| Box::new(FsIoError::RmDir(path.as_ref().into(), e)))
    }

    pub fn remove_dir<P: AsRef<Utf8Path>>(path: P) -> Result<(), Box<FsIoError>> {
        fs::remove_dir(path.as_ref())
            .map_err(|e| Box::new(FsIoError::RmDir(path.as_ref().into(), e)))
    }

    pub fn remove_file<P: AsRef<Utf8Path>>(path: P) -> Result<(), Box<FsIoError>> {
        fs::remove_file(path.as_ref())
            .map_err(|e| Box::new(FsIoError::RmFile(path.as_ref().into(), e)))
    }

    pub fn copy<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
        from: P,
        to: Q,
    ) -> Result<u64, Box<FsIoError>> {
        fs::copy(from.as_ref(), to.as_ref()).map_err(|e| {
            Box::new(FsIoError::CopyFile(
                from.as_ref().into(),
                to.as_ref().into(),
                e,
            ))
        })
    }

    pub fn read_to_string<P: AsRef<Utf8Path>>(path: P) -> Result<String, Box<FsIoError>> {
        fs::read_to_string(path.as_ref())
            .map_err(|e| Box::new(FsIoError::ReadFile(path.as_ref().into(), e)))
    }

    pub fn read<P: AsRef<Utf8Path>>(path: P) -> Result<Vec<u8>, Box<FsIoError>> {
        fs::read(path.as_ref()).map_err(|e| Box::new(FsIoError::ReadFile(path.as_ref().into(), e)))
    }

    pub fn metadata<P: AsRef<Utf8Path>>(path: P) -> Result<fs::Metadata, Box<FsIoError>> {
        fs::metadata(path.as_ref())
            .map_err(|e| Box::new(FsIoError::Metadata(path.as_ref().into(), e)))
    }

    pub fn write<P: AsRef<Utf8Path>, C: AsRef<[u8]>>(
        path: P,
        contents: C,
    ) -> Result<(), Box<FsIoError>> {
        fs::write(path.as_ref(), contents)
            .map_err(|e| Box::new(FsIoError::WriteFile(path.as_ref().into(), e)))
    }

    /// Canonicalizes UTF-8 path. If canonicalized path is not valid
    /// UTF-8, returns `io::Error` of `InvalidData` kind.
    /// On Windows this returns most compatible form of a path instead of UNC.
    pub fn canonicalize<P: AsRef<Utf8Path>>(path: P) -> Result<Utf8PathBuf, Box<FsIoError>> {
        canonicalize_raw(path.as_ref())
            .map_err(|e| Box::new(FsIoError::Canonicalize(path.as_ref().into(), e)))
    }

    /// Same as `canonicalize`, with the raw `io::Error`
    pub fn canonicalize_raw<P: AsRef<Utf8Path>>(path: P) -> Result<Utf8PathBuf, io::Error> {
        let path = dunce::canonicalize(path.as_ref())?;
        Utf8PathBuf::from_path_buf(path)
            // error is just original `path`
            .map_err(|_path| io::Error::from(io::ErrorKind::InvalidData))
    }

    /// see `std::path::absolute()`
    pub fn absolute<P: AsRef<Utf8Path>>(path: P) -> Result<Utf8PathBuf, Box<FsIoError>> {
        camino::absolute_utf8(path.as_ref())
            .map_err(|e| Box::new(FsIoError::Absolute(path.as_ref().into(), e)))
    }

    /// Get current dir as UTF-8 path. If current dir path is not valid
    /// UTF-8, returns `io::Error` of `InvalidData` kind.
    pub fn current_dir() -> Result<Utf8PathBuf, Box<FsIoError>> {
        std::env::current_dir()
            .and_then(|d| {
                Utf8PathBuf::from_path_buf(d)
                    // error is just original `path`
                    .map_err(|_path| io::Error::from(io::ErrorKind::InvalidData))
            })
            .map_err(|e| Box::new(FsIoError::CurrentDir(e)))
    }

    /// Returns `true` if the given path exists and is a regular file.
    ///
    /// This function attempts to retrieve the metadata for `path` and checks
    /// whether it represents a regular file.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the path exists and is a regular file.
    /// - `Ok(false)` if the path does not exist or it is not a regular file.
    /// - `Err(_)` if an I/O error occurs while retrieving metadata for reasons
    ///   other than the path not being found (e.g., permission denied).
    ///
    /// # Errors
    ///
    /// Returns an [`FsIoError`] if metadata retrieval fails for any reason
    /// other than [`std::io::ErrorKind::NotFound`].
    pub fn is_file<P: AsRef<Utf8Path>>(path: P) -> Result<bool, Box<FsIoError>> {
        match fs::metadata(path.as_ref()) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(Box::new(FsIoError::IsFile(path.as_ref().into(), err))),
        }
    }

    /// Returns `true` if the given path exists and is a directory.
    ///
    /// This function attempts to retrieve the metadata for `path` and checks
    /// whether it represents a directory.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the path exists and is a directory.
    /// - `Ok(false)` if the path does not exist or it is not a directory.
    /// - `Err(_)` if an I/O error occurs while retrieving metadata for reasons
    ///   other than the path not being found (e.g., permission denied).
    ///
    /// # Errors
    ///
    /// Returns an [`FsIoError`] if metadata retrieval fails for any reason
    /// other than [`std::io::ErrorKind::NotFound`].
    pub fn is_dir<P: AsRef<Utf8Path>>(path: P) -> Result<bool, Box<FsIoError>> {
        match fs::metadata(path.as_ref()) {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(Box::new(FsIoError::IsFile(path.as_ref().into(), err))),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProjectDeserializationError {
    /// The document is not json, or a value in it has the wrong type.
    /// Serde knows where, so it reports it.
    #[error("project deserialization error: failed to deserialize `{path}`")]
    Syntax {
        path: Box<Utf8Path>,
        #[source]
        err: serde_json::Error,
    },
    /// The document is json, but not a manifest: a key it must carry is
    /// absent. Every reason is reported at once.
    #[error("`{}` is not a valid manifest:\n{}", .report.file(), indent(report))]
    Invalid { report: Report },
}

impl ProjectDeserializationError {
    pub fn new(path: impl AsRef<Utf8Path>, err: serde_json::Error) -> Self {
        Self::Syntax {
            path: path.as_ref().into(),
            err,
        }
    }

    pub fn invalid(path: impl AsRef<Utf8Path>, diagnostics: impl Into<Diagnostics>) -> Self {
        Self::Invalid {
            report: diagnostics.into().in_file(path.as_ref().as_str()),
        }
    }
}

/// Takes a validated manifest, logging every warning its read produced
/// against the document it came from.
pub fn validate_manifest<T>(
    path: impl AsRef<Utf8Path>,
    validated: Result<Validated<T>, Diagnostics>,
) -> Result<T, ProjectDeserializationError> {
    match validated {
        Ok(validated) => {
            for warning in &validated.warnings {
                log::warn!("{}", warning.in_file(path.as_ref().as_str()));
            }
            Ok(validated.value)
        }
        Err(report) => Err(ProjectDeserializationError::invalid(path, report)),
    }
}

/// Indent every line of a multi-line report to align with the first.
fn indent(report: &Report) -> String {
    report
        .to_string()
        .lines()
        .map(|line| format!("{SP:>8}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Error)]
#[error("project serialization error: {msg}")]
pub struct ProjectSerializationError {
    msg: String,
    #[source]
    err: serde_json::Error,
}

impl ProjectSerializationError {
    pub fn new(msg: String, err: serde_json::Error) -> Self {
        Self { msg, err }
    }
}

/// All zip errors we use
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum ZipArchiveError {
    #[error("failed to parse zip archive `{0}`")]
    ReadArchive(Box<Utf8Path>, #[source] ZipError),
    #[error("failed to retrieve file from zip archive")]
    FileMeta(#[source] ZipError),
    #[error("failed to retrieve file from zip archive: {0}")]
    NamedFileMeta(Box<str>, ZipError),
    #[error(
        "zip archive path handling error: file `{0}` in a zip archive is not contained in a directory"
    )]
    InvalidPath(Box<Utf8Path>),
    #[error("failed to write file `{0}` to zip archive")]
    Write(Box<Utf8Path>, #[source] ZipError),
    #[error("failed to finish creating zip archive at `{0}`")]
    Finish(Box<Utf8Path>, #[source] ZipError),
}

/// Project identifier IRI. Always a valid IRI.
// TODO: construction steps
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub struct Identifier(String);

impl From<&Identifier> for toml_edit::Value {
    fn from(val: &Identifier) -> Self {
        Self::from(&val.0)
    }
}

impl Borrow<str> for Identifier {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Borrow<String> for Identifier {
    fn borrow(&self) -> &String {
        &self.0
    }
}

impl From<&InterchangeProjectUsage> for Identifier {
    fn from(value: &InterchangeProjectUsage) -> Self {
        match value {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            }
            | InterchangeProjectUsage::KparUrl {
                kpar_url: resource,
                publisher: _,
                name: _,
            } => Self(resource.to_string()),
            InterchangeProjectUsage::Directory {
                publisher,
                name,
                dir: _,
            }
            | InterchangeProjectUsage::KparPath {
                publisher,
                name,
                kpar_path: _,
            } => Self::from_project(publisher, name),
            InterchangeProjectUsage::Index {
                publisher,
                name,
                version_constraint: _,
            } => Self::from_index(publisher, name),
        }
    }
}

impl From<InterchangeProjectUsage> for Identifier {
    fn from(value: InterchangeProjectUsage) -> Self {
        match value {
            InterchangeProjectUsage::Resource {
                resource,
                version_constraint: _,
            }
            | InterchangeProjectUsage::KparUrl {
                kpar_url: resource,
                publisher: _,
                name: _,
            } => Self(resource.into_string()),
            typed => Self::from(&typed),
        }
    }
}

/// The prefix of the identifiers Sysand derives from publishers and names
/// that are not spelled as an index usage can spell them (see
/// [`Identifier::from_project`]). These are internal to Sysand and never
/// written by users
pub const URN_SYSAND_PREFIX: &str = "urn:sysand:";

/// Whether `iri` is an `urn:sysand` IRI. Both the URN scheme and namespace
/// are case-insensitive (RFC 8141)
pub fn is_urn_sysand(iri: &str) -> bool {
    iri.get(..URN_SYSAND_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(URN_SYSAND_PREFIX))
}

impl Identifier {
    /// The identifier of the project spelled `publisher`/`name`, which are
    /// validated here; callers that hold a [`ProjectPublisher`] and
    /// [`ProjectName`] should use [`Self::from_project`]
    pub fn from_pub_name(publisher: &str, name: &str) -> Self {
        Self::make_identifier_iri(publisher, name)
    }

    /// The `pkg:sysand` PURL of the project spelled `publisher`/`name`, as an
    /// index usage spells it; always valid, so nothing has to be checked
    pub fn from_index(publisher: &IndexPublisher, name: &IndexName) -> Self {
        Self(sysand_purl(&publisher.normalized(), &name.normalized()))
    }

    /// The identifier of the project spelled `publisher`/`name`: as
    /// [`Self::from_index`] makes it if an index usage can spell it so,
    /// otherwise a `urn:sysand` IRI of the normalized publisher and name
    ///
    /// Which of the two is decided by the spelling as given, not by the
    /// normalized form: e.g. `ACME Inc./Foo` (not an index spelling, due to
    /// the `.`) is `urn:sysand:acme-inc/foo`, although its normalized form
    /// `acme-inc/foo` is a valid index spelling, which is
    /// `pkg:sysand/acme-inc/foo`. So a directory or KPAR usage of
    /// `ACME Inc./Foo` and an index usage of `acme-inc/foo` are different
    /// projects.
    pub fn from_project(publisher: &ProjectPublisher, name: &ProjectName) -> Self {
        if let (Ok(publisher), Ok(name)) = (
            IndexPublisher::parse(publisher.as_str().to_owned()),
            IndexName::parse(name.as_str().to_owned()),
        ) {
            return Self::from_index(&publisher, &name);
        }
        Self::urn(publisher.normalized(), name.normalized())
    }

    /// The `urn:sysand` IRI of `publisher`/`name`
    fn urn(publisher: &str, name: &str) -> Self {
        let mut enc_pub = EString::<IData>::new();
        enc_pub.encode_str::<IData>(publisher);
        let mut enc_name = EString::<IData>::new();
        enc_name.encode_str::<IData>(name);
        Self(format!("{URN_SYSAND_PREFIX}{enc_pub}/{enc_name}"))
    }

    /// The (possibly not well-formed) identifier of a usage that has *not*
    /// been validated, e.g. one read straight out of a `.project.json` document.
    /// Never persist in files! Can only be used for comparison with other
    /// identifiers.
    ///
    /// Returns `None` for a typed usage with an empty `publisher` or `name`,
    /// which has no well-formed identifier (and which
    /// [`Self::from_interchange_usage_unchecked`] rejects with a debug
    /// assertion). Usages of *any* kind can be compared by identity
    /// without trusting the manifest first.
    pub fn from_unvalidated_usage(usage: &InterchangeProjectUsageRaw) -> Option<Self> {
        if usage
            .typed_publisher_name()
            .is_some_and(|(publisher, name)| publisher.is_empty() || name.is_empty())
        {
            return None;
        }
        Some(Self::from_interchange_usage_unchecked(usage))
    }

    pub fn from_interchange_usage_unchecked(usage: &InterchangeProjectUsageRaw) -> Self {
        match usage {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint: _,
            }
            | InterchangeProjectUsageRaw::KparUrl {
                kpar_url: resource,
                publisher: _,
                name: _,
            } => Self(resource.clone()),
            typed => {
                let (publisher, name) = typed
                    .typed_publisher_name()
                    .expect("a non-resource usage is typed");
                Self::make_identifier_iri(publisher, name)
            }
        }
    }

    pub fn from_iri_owned(iri: Iri<String>) -> Self {
        Self(iri.into_string())
    }

    pub fn from_iri(iri: &Iri<&str>) -> Self {
        Self(iri.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Construct `Identifier` from a String, assuming it's a valid IRI
    pub fn from_iri_unchecked(iri: String) -> Self {
        Self(iri)
    }

    /// Construct `Identifier` from `&str`, assuming it's a valid IRI
    pub fn from_iri_unchecked_str(iri: &str) -> Self {
        Self(iri.to_owned())
    }

    fn make_identifier_iri(publisher: impl AsRef<str>, name: impl AsRef<str>) -> Self {
        let publisher = publisher.as_ref();
        let name = name.as_ref();
        debug_assert_ne!(publisher, "");
        debug_assert_ne!(name, "");

        match (
            ProjectPublisher::parse(publisher.to_owned()),
            ProjectName::parse(name.to_owned()),
        ) {
            (Ok(publisher), Ok(name)) => Self::from_project(&publisher, &name),
            // Not a valid project spelling (only from an unvalidated usage):
            // what of it is valid normalized, the rest as it is
            (publisher, name) => {
                let publisher = publisher.map_or_else(
                    |(publisher, _)| publisher,
                    ProjectPublisher::into_normalized,
                );
                let name = name.map_or_else(|(name, _)| name, ProjectName::into_normalized);
                Self::urn(&publisher, &name)
            }
        }
    }
}

impl AsRef<str> for Identifier {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for Identifier {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Iri<String>> for Identifier {
    fn from(value: Iri<String>) -> Self {
        Self(value.into_string())
    }
}

impl Display for Identifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a mismatch spells one side of a comparison: `<publisher>/<name>`,
/// with `<none>` for a side that declares no publisher.
#[cfg_attr(
    not(feature = "filesystem"),
    expect(
        dead_code,
        reason = "the readers that render a spelling mismatch are behind `filesystem`"
    )
)]
pub(crate) fn pair(publisher: Option<&str>, name: &str) -> String {
    match publisher {
        Some(publisher) => format!("{publisher}/{name}"),
        None => format!("<none>/{name}"),
    }
}

#[cfg(test)]
#[path = "./utils_tests.rs"]
mod tests;
