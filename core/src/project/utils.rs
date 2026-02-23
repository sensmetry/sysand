// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
#[cfg(feature = "filesystem")]
use zip::{self, result::ZipError};

use std::io::{self, Read};

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

/// The errors arising from filesystem I/O.
/// The variants defined here include relevant context where possible.
#[derive(Error, Debug)]
pub enum FsIoError {
    #[error("failed to canonicalize path\n  `{0}`:\n  {1}")]
    Canonicalize(Utf8PathBuf, io::Error),
    #[error("failed to create directory\n  `{0}`:\n  {1}")]
    MkDir(Utf8PathBuf, io::Error),
    #[error("failed to open file\n  `{0}`:\n  {1}")]
    OpenFile(Utf8PathBuf, io::Error),
    #[error("failed to get metadata for\n  `{0}`:\n  {1}")]
    Metadata(Utf8PathBuf, io::Error),
    /// Same as `Self::Metadata`, but path is unknown when using file handle
    #[error("failed to get metadata for file: {0}")]
    MetadataHandle(io::Error),
    #[error("failed to create a temporary file: {0}")]
    CreateTempFile(io::Error),
    #[error("failed to create a temporary directory: {0}")]
    MkTempDir(io::Error),
    #[error("failed to write file\n  `{0}`:\n  {1}")]
    WriteFile(Utf8PathBuf, io::Error),
    #[error("failed to read directory\n  `{0}`:\n  {1}")]
    ReadDir(Utf8PathBuf, io::Error),
    #[error("failed to read file\n  `{0}`:\n  {1}")]
    ReadFile(Utf8PathBuf, io::Error),
    /// Same as `Self::ReadFile`, but path is unknown when using file handle.
    #[error("failed to read file: {0}")]
    ReadFileHandle(io::Error),
    #[error("failed to move\n  `{0}` to\n  `{1}`:\n  {2}")]
    Move(Utf8PathBuf, Utf8PathBuf, io::Error),
    #[error("failed to create file\n  `{0}`:\n  {1}")]
    CreateFile(Utf8PathBuf, io::Error),
    #[error("failed to copy file from\n  `{0}` to\n  `{1}`:\n  {2}")]
    CopyFile(Utf8PathBuf, Utf8PathBuf, io::Error),
    #[error("failed to remove file\n  `{0}`:\n  {1}")]
    RmFile(Utf8PathBuf, io::Error),
    #[error("failed to remove directory\n  `{0}`:\n  {1}")]
    RmDir(Utf8PathBuf, io::Error),
    #[error("failed to get path to current directory:\n  {0}")]
    CurrentDir(io::Error),
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

    #[allow(non_snake_case)]
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

    pub fn canonicalize<P: AsRef<Utf8Path>>(path: P) -> Result<Utf8PathBuf, Box<FsIoError>> {
        path.as_ref()
            .canonicalize_utf8()
            .map_err(|e| Box::new(FsIoError::Canonicalize(path.as_ref().into(), e)))
    }

    /// see `std::path::absolute()`
    pub fn absolute<P: AsRef<Utf8Path>>(path: P) -> Result<Utf8PathBuf, Box<FsIoError>> {
        camino::absolute_utf8(path.as_ref())
            .map_err(|e| Box::new(FsIoError::Canonicalize(path.as_ref().into(), e)))
    }

    /// Get current dir as UTF-8 path. If current dir path is not valid
    /// UTF-8, returns `io::Error` of `InvalidData` kind.
    pub fn current_dir() -> Result<Utf8PathBuf, Box<FsIoError>> {
        std::env::current_dir()
            .and_then(|d| {
                Utf8PathBuf::from_path_buf(d)
                    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
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
        match metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(err) if matches!(err.as_ref(), FsIoError::Metadata(_, e) if e.kind() == ErrorKind::NotFound) => {
                Ok(false)
            }
            Err(err) => Err(err),
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
        match metadata(path) {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(err) if matches!(err.as_ref(), FsIoError::Metadata(_, e) if e.kind() == ErrorKind::NotFound) => {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }
}

#[derive(Debug, Error)]
#[error("project deserialization error: {msg}: {err}")]
pub struct ProjectDeserializationError {
    msg: &'static str,
    err: serde_json::Error,
}

impl ProjectDeserializationError {
    pub fn new(msg: &'static str, err: serde_json::Error) -> Self {
        Self { msg, err }
    }
}

#[derive(Debug, Error)]
#[error("project serialization error: {msg}: {err}")]
pub struct ProjectSerializationError {
    msg: String,
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
    #[error("failed to parse zip archive `{0}`: {1}")]
    ReadArchive(Box<Utf8Path>, ZipError),
    #[error("failed to retrieve file from zip archive: {0}")]
    FileMeta(ZipError),
    #[error("failed to retrieve file from zip archive: {0}")]
    NamedFileMeta(Box<str>, ZipError),
    #[error(
        "zip archive path handling error: file `{0}` in a zip archive is not contained in a directory"
    )]
    InvalidPath(Box<Utf8Path>),
    #[error("failed to write file `{0}` to zip archive: {1}")]
    Write(Box<Utf8Path>, ZipError),
    #[error("failed to finish creating zip archive at `{0}`: {1}")]
    Finish(Box<Utf8Path>, ZipError),
}
