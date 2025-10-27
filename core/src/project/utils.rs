// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
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
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.internal.read(buf)
    }
}

pub trait ToPathBuf {
    fn to_path_buf(&self) -> PathBuf;
}

impl ToPathBuf for std::path::Path {
    fn to_path_buf(&self) -> PathBuf {
        self.into()
    }
}

impl<P> ToPathBuf for P
where
    P: AsRef<Path>,
{
    fn to_path_buf(&self) -> PathBuf {
        self.as_ref().into()
    }
}

/// The errors arising from filesystem I/O.
/// The variants defined here include relevant context where possible.
#[derive(Error, Debug)]
pub enum FsIoError {
    #[error("failed to canonicalize path\n  '{0}':\n  {1}")]
    Canonicalize(PathBuf, io::Error),
    #[error("failed to create directory\n  '{0}':\n  {1}")]
    MkDir(PathBuf, io::Error),
    #[error("failed to open file\n  '{0}':\n  {1}")]
    OpenFile(PathBuf, io::Error),
    #[error("failed to get metadata for\n  '{0}':\n  {1}")]
    Metadata(PathBuf, io::Error),
    /// Failed to get metadata from file handle and path is unknown
    /// at call location.
    #[error("failed to get metadata for file: {0}")]
    MetadataHandle(io::Error),
    #[error("failed to create a temporary file: {0}")]
    CreateTempFile(io::Error),
    #[error("failed to create a temporary directory: {0}")]
    MkTempDir(io::Error),
    #[error("failed to write file\n  '{0}':\n  {1}")]
    WriteFile(PathBuf, io::Error),
    #[error("failed to read directory\n  '{0}':\n  {1}")]
    ReadDir(PathBuf, io::Error),
    #[error("failed to read file\n  '{0}':\n  {1}")]
    ReadFile(PathBuf, io::Error),
    /// Path is unknown when reading a file from handle.
    #[error("failed to read file: {0}")]
    ReadFileHandle(io::Error),
    #[error("failed to move\n  '{0}' to\n  '{1}':\n  {2}")]
    Move(PathBuf, PathBuf, io::Error),
    #[error("failed to create file\n  '{0}':\n  {1}")]
    CreateFile(PathBuf, io::Error),
    #[error("failed to copy file from\n  '{0}' to\n  '{1}':\n  {2}")]
    CopyFile(PathBuf, PathBuf, io::Error),
    #[error("failed to remove file\n  '{0}':\n  {1}")]
    RmFile(PathBuf, io::Error),
    #[error("failed to remove directory\n  '{0}':\n  {1}")]
    RmDir(PathBuf, io::Error),
}

/// Wrappers for filesystem I/O functions to return `FsIoError`.
/// Copies the `std` interface 1 to 1, except for the error type.
pub mod wrapfs {
    use std::fs;
    use std::path;
    use std::path::Path;

    use super::FsIoError;
    use super::ToPathBuf;

    #[allow(non_snake_case)]
    pub mod File {
        use std::fs;
        use std::path::Path;

        use super::FsIoError;
        use super::ToPathBuf;

        pub fn open<P: AsRef<Path>>(path: P) -> Result<fs::File, FsIoError> {
            fs::File::open(&path).map_err(|e| FsIoError::OpenFile(path.to_path_buf(), e))
        }

        pub fn create<P: AsRef<Path>>(path: P) -> Result<fs::File, FsIoError> {
            fs::File::create(&path).map_err(|e| FsIoError::CreateFile(path.to_path_buf(), e))
        }
    }

    pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<(), FsIoError> {
        fs::create_dir(&path).map_err(|e| FsIoError::MkDir(path.to_path_buf(), e))
    }

    pub fn create_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FsIoError> {
        fs::create_dir_all(&path).map_err(|e| FsIoError::MkDir(path.to_path_buf(), e))
    }

    pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<fs::ReadDir, FsIoError> {
        fs::read_dir(&path).map_err(|e| FsIoError::ReadDir(path.to_path_buf(), e))
    }

    pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<(), FsIoError> {
        fs::remove_dir_all(&path).map_err(|e| FsIoError::RmDir(path.to_path_buf(), e))
    }

    pub fn remove_dir<P: AsRef<Path>>(path: P) -> Result<(), FsIoError> {
        fs::remove_dir(&path).map_err(|e| FsIoError::RmDir(path.to_path_buf(), e))
    }

    pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), FsIoError> {
        fs::remove_file(&path).map_err(|e| FsIoError::RmFile(path.to_path_buf(), e))
    }

    pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<u64, FsIoError> {
        fs::copy(&from, &to)
            .map_err(|e| FsIoError::CopyFile(from.to_path_buf(), to.to_path_buf(), e))
    }

    pub fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String, FsIoError> {
        fs::read_to_string(&path).map_err(|e| FsIoError::ReadFile(path.to_path_buf(), e))
    }

    pub fn metadata<P: AsRef<Path>>(path: P) -> Result<fs::Metadata, FsIoError> {
        fs::metadata(&path).map_err(|e| FsIoError::Metadata(path.to_path_buf(), e))
    }

    pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> Result<(), FsIoError> {
        fs::write(&path, contents).map_err(|e| FsIoError::WriteFile(path.to_path_buf(), e))
    }

    pub fn canonicalize<P: AsRef<Path>>(path: P) -> Result<path::PathBuf, FsIoError> {
        fs::canonicalize(&path).map_err(|e| FsIoError::Canonicalize(path.to_path_buf(), e))
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
    msg: &'static str,
    err: serde_json::Error,
}

impl ProjectSerializationError {
    pub fn new(msg: &'static str, err: serde_json::Error) -> Self {
        Self { msg, err }
    }
}
