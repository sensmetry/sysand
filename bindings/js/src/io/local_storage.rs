// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::{Cursor, Read};

use sysand_core::{
    lock,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectMut, ProjectRead, utils::FsIoError},
};

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::local_storage_utils::{LocalStorageError, LocalStorageVFS, get_local_browser_storage};

#[derive(Debug)]
pub struct ProjectLocalBrowserStorage {
    pub root_path: Utf8UnixPathBuf,
    pub vfs: LocalStorageVFS,
}

pub fn open_project_local_storage<S: AsRef<str>, P: AsRef<Utf8UnixPath>>(
    prefix: S,
    root_path: P,
) -> Result<ProjectLocalBrowserStorage, Error> {
    Ok(ProjectLocalBrowserStorage {
        root_path: root_path.as_ref().to_path_buf(),
        vfs: get_local_browser_storage(prefix)?,
    })
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("refusing to overwrite already existing `{0}`")]
    AlreadyExists(Box<str>),
    #[error("failed to get `window` object")]
    NoWindow,
    #[error("failed to get `window.localStorage` object")]
    NoLocalStorage,
    #[error("JS error: {0:?}")]
    Js(wasm_bindgen::JsValue),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to serialize information to write it to `{0}`: {1}")]
    Serialize(&'static str, serde_json::Error),
    #[error("failed to serialize: {0}")]
    SerializeHandle(#[from] serde_json::Error),
    #[error("key `{0}` not found in local storage")]
    KeyNotFound(String),
}

impl From<FsIoError> for Error {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalStorageError> for Error {
    fn from(value: LocalStorageError) -> Self {
        match value {
            LocalStorageError::NoWindow => Error::NoWindow,
            LocalStorageError::NoLocalStorage => Error::NoLocalStorage,
            LocalStorageError::Js(js_value) => Error::Js(js_value),
            LocalStorageError::Io(error) => Error::Io(error),
            LocalStorageError::Serialize(error) => Error::SerializeHandle(error),
            LocalStorageError::KeyNotFound(key) => Error::KeyNotFound(key),
        }
    }
}

impl ProjectRead for ProjectLocalBrowserStorage {
    type Error = Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let info_path = self.root_path.join(".project.json");

        let info = match self.vfs.read_string(info_path) {
            Ok(str) => {
                Some(serde_json::from_str(&str).map_err(|e| Error::Serialize(".project.json", e))?)
            }
            Err(LocalStorageError::KeyNotFound(_)) => None,
            Err(err) => {
                return Err(err.into());
            }
        };

        let meta_path = self.root_path.join(".meta.json");

        let meta = match self.vfs.read_string(meta_path) {
            Ok(str) => {
                Some(serde_json::from_str(&str).map_err(|e| Error::Serialize(".meta.json", e))?)
            }
            Err(LocalStorageError::KeyNotFound(_)) => None,
            Err(err) => {
                return Err(err.into());
            }
        };

        Ok((info, meta))
    }

    type SourceReader<'a> = Cursor<String>;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        Ok(Cursor::new(
            self.vfs.read_string(self.root_path.join(path))?,
        ))
    }

    fn sources(&self) -> Vec<lock::Source> {
        vec![sysand_core::lock::Source::LocalSrc {
            src_path: self.root_path.to_string(),
        }]
    }
}

impl ProjectMut for ProjectLocalBrowserStorage {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        let info_path = self.root_path.join(".project.json");
        if !overwrite && self.vfs.exists(&info_path)? {
            return Err(Error::AlreadyExists(".project.json".into()));
        }

        self.vfs.write_serialisable(&info_path, info)?;

        Ok(())
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        let meta_path = self.root_path.join(".meta.json");
        if !overwrite && self.vfs.exists(&meta_path)? {
            return Err(Error::AlreadyExists(".meta.json".into()));
        }

        self.vfs.write_serialisable(
            &meta_path,
            Into::<InterchangeProjectMetadataRaw>::into(meta.to_owned()),
        )?;

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        let file_path = self.root_path.join(&path);

        if !overwrite && self.vfs.exists(&file_path)? {
            return Err(Error::AlreadyExists(file_path.as_str().into()));
        }

        self.vfs.write_reader(&file_path, source)?;

        Ok(())
    }
}
