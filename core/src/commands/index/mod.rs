// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    fs::File,
    io::{ErrorKind, Read, Seek, Write as _},
};

use camino::Utf8Path;

mod add;
mod init;
mod remove;
mod yank;

pub use add::do_index_add;
pub use init::do_index_init;
pub use remove::do_index_remove;
pub use yank::do_index_yank;

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::project::utils::FsIoError;

pub const INDEX_FILE_NAME: &str = "index.json";
pub const VERSIONS_FILE_NAME: &str = "versions.json";
pub const KPAR_FILE_NAME: &str = "project.kpar";
pub const INFO_FILE_NAME: &str = ".project.json";
pub const META_FILE_NAME: &str = ".meta.json";

#[derive(Error, Debug)]
pub(crate) enum JsonFileError {
    #[error(transparent)]
    FileDoesNotExist(Box<FsIoError>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("patching json `{path}` failed as the current contents are invalid")]
    InvalidJsonFile {
        path: Box<str>,
        #[source]
        source: serde_json::Error,
    },
}
pub(crate) fn open_json_file<T: Default + Serialize + DeserializeOwned>(
    path: &Utf8Path,
    create: bool,
) -> Result<(File, T), JsonFileError> {
    let mut file = File::options()
        .create(create)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| {
            let err_kind = e.kind();
            let fs_io_err = Box::new(FsIoError::OpenFile(path.to_owned(), e));
            match err_kind {
                ErrorKind::NotFound => JsonFileError::FileDoesNotExist(fs_io_err),
                _ => JsonFileError::Io(fs_io_err),
            }
        })?;
    let mut file_contents = String::new();
    file.read_to_string(&mut file_contents)
        .map_err(|e| Box::new(FsIoError::ReadFile(path.to_owned(), e)))?;
    let value = if file_contents.is_empty() {
        T::default()
    } else {
        serde_json::from_str(&file_contents).map_err(|e| JsonFileError::InvalidJsonFile {
            path: path.as_str().into(),
            source: e,
        })?
    };

    Ok((file, value))
}

pub(crate) fn to_json_string<T: Serialize>(value: &T) -> String {
    // If this fails, it's a bug
    serde_json::to_string_pretty(value).unwrap()
}

pub(crate) fn overwrite_file(
    file: &mut File,
    path: &Utf8Path,
    contents: &str,
) -> Result<(), Box<FsIoError>> {
    let map_err = |e| Box::new(FsIoError::WriteFile(path.into(), e));
    // Without this the new content would be appended to the end of the file if the file was read first
    file.rewind().map_err(map_err)?;
    // Without this if the file was longer previously, only the start of it would be overwritten
    file.set_len(0).map_err(map_err)?;
    file.write_all(contents.as_bytes()).map_err(map_err)
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
