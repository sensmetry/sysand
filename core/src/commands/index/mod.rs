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

// pub fn index_path() -> Utf8PathBuf {
//     Utf8PathBuf::from(INDEX_PATH)
// }

// pub fn versions_path(project_path: &Utf8Path) -> Utf8PathBuf {
//     project_path.join("versions.json")
// }

// pub fn project_dir<S: AsRef<str>>(project_path: &Utf8Path, version: S) -> Utf8PathBuf {
//     project_path.join(version.as_ref()).join("project.kpar")
// }

// pub fn index_kpar_path<S: AsRef<str>>(project_path: &Utf8Path, version: S) -> Utf8PathBuf {
//     project_path.join(version.as_ref()).join("project.kpar")
// }

pub(crate) const NOT_AN_INDEX_MESSAGE: &str = "current directory is not an index as it doesn't have index.json file; make sure you run `sysand index init` in this directory before adding any packages";

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
    // if let Some(dir) = path.parent() {
    //     wrapfs::create_dir_all(dir).map_err(|e| JsonFileError::CreateDirectory {
    //         path: path.as_str().into(),
    //         source: e.into(),
    //     })?;
    // }
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
    // TODO(JP): ask if all these are necessary to override the current file contents
    file.set_len(0).map_err(map_err)?;
    file.rewind().map_err(map_err)?;
    file.write_all(contents.as_bytes()).map_err(map_err)
}

// #[derive(Error, Debug)]
// pub enum JsonFilePatchError {
//     #[error("unable to create the directory for {path}")]
//     CreateDirectory {
//         path: Box<str>,
//         #[source]
//         source: Box<FsIoError>,
//     },
//     #[error(transparent)]
//     Io(#[from] Box<FsIoError>),
//     #[error("patching json {path} failed as the existing json is invalid")]
//     InvalidExistingJson {
//         path: Box<str>,
//         #[source]
//         source: serde_json::Error,
//     },
//     #[error("failed to serialize {path}")]
//     Serialize {
//         path: Box<str>,
//         #[source]
//         source: serde_json::Error,
//     },
// }

// pub fn patch_json_file<P: AsRef<Utf8Path>, T: Default + Serialize + DeserializeOwned>(
//     path: P,
//     patch: fn(&mut T),
// ) -> Result<(), JsonFilePatchError> {
//     let path = path.as_ref();
//     if let Some(dir) = path.parent() {
//         wrapfs::create_dir_all(dir).map_err(|e| JsonFilePatchError::CreateDirectory {
//             path: path.as_str().into(),
//             source: e.into(),
//         })?;
//     }
//     let mut file = File::options()
//         .create(true)
//         .read(true)
//         .write(true)
//         .open(path)
//         .map_err(|e| Box::new(FsIoError::OpenFile(path.to_owned(), e)))?;
//     let mut file_contents;
//     file.read_to_string(file_contents)
//         .map_err(|e| Box::new(FsIoError::ReadFile(path.to_owned(), e)))?;
//     let mut value = if file_contents.is_empty() {
//         T::default()
//     } else {
//         serde_json::from_str(&file_contents).map_err(|e| {
//             JsonFilePatchError::InvalidExistingJson {
//                 path: path.as_str().into(),
//                 source: e,
//             }
//         })?
//     };
//     patch(&mut value);
//     let file_contents =
//         serde_json::to_string_pretty(&value).map_err(|e| JsonFilePatchError::Serialize {
//             path: path.as_str().into(),
//             source: e,
//         })?;
//     file.write_all(file_contents.as_bytes())
//         .map_err(|e| Box::new(FsIoError::WriteFile(path.to_owned(), e)))?;
//     Ok(())
//     // wrapfs::File::open(path)?;
//     // if file.i

//     // let path = path.as_ref();
//     // let mut versions = match wrapfs::read_to_string(path) {
//     //     Ok(versions_str) => serde_json::from_str(&versions_str).map_err(|e| {
//     //         JsonFilePatchError::InvalidExistingJson {
//     //             path: path.as_str().into(),
//     //             source: e,
//     //         }
//     //     })?,
//     //     Err(_) => T::default(),
//     // };
//     // patch(&mut versions);
//     // let versions_str = serde_json::to_string(value);
//     // wrapfs::write(path, contents)
// }

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
