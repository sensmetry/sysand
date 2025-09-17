// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Cursor;

use sysand_core::{
    model::InterchangeProjectMetadataRaw,
    project::{ProjectMut, ProjectRead},
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
    #[error("refusing to overwrite")]
    AlreadyExists(String),
    #[error("failed to get window")]
    NoWindow(),
    #[error("failed to get local storage")]
    NoLocalStorage(),
    #[error("JS error")]
    JSError(wasm_bindgen::JsValue),
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("de/serialisation error")]
    SerialisationError(#[from] serde_json::Error),
    #[error("key not found")]
    KeyNotFoundError(),
}

impl From<LocalStorageError> for Error {
    fn from(value: LocalStorageError) -> Self {
        match value {
            LocalStorageError::NoWindow() => Error::NoWindow(),
            LocalStorageError::NoLocalStorage() => Error::NoLocalStorage(),
            LocalStorageError::JSError(js_value) => Error::JSError(js_value),
            LocalStorageError::IOError(error) => Error::IOError(error),
            LocalStorageError::SerialisationError(error) => Error::SerialisationError(error),
            LocalStorageError::KeyNotFoundError() => Error::KeyNotFoundError(),
        }
    }
}

impl ProjectRead for ProjectLocalBrowserStorage {
    type Error = Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<sysand_core::model::InterchangeProjectInfoRaw>,
            Option<sysand_core::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let info_path = self.root_path.join(".project.json");

        let info = match self.vfs.read_string(info_path) {
            Ok(str) => Some(serde_json::from_str(&str)?),
            Err(LocalStorageError::KeyNotFoundError()) => None,
            Err(err) => {
                return Err(err.into());
            }
        };

        let meta_path = self.root_path.join(".meta.json");

        let meta = match self.vfs.read_string(meta_path) {
            Ok(str) => Some(serde_json::from_str(&str)?),
            Err(LocalStorageError::KeyNotFoundError()) => None,
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

    fn sources(&self) -> Vec<sysand_core::lock::Source> {
        vec![sysand_core::lock::Source::LocalSrc {
            src_path: self.root_path.to_string(),
        }]
    }
}

impl ProjectMut for ProjectLocalBrowserStorage {
    fn put_info(
        &mut self,
        info: &sysand_core::model::InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        let info_path = self.root_path.join(".project.json");
        if !overwrite && self.vfs.exists(&info_path)? {
            return Err(Error::AlreadyExists(
                ".project.json already exists".to_string(),
            ));
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
            return Err(Error::AlreadyExists(
                ".meta.json already exists".to_string(),
            ));
        }

        self.vfs.write_serialisable(
            &meta_path,
            Into::<InterchangeProjectMetadataRaw>::into(meta.to_owned()),
        )?;

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: std::io::Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        let file_path = self.root_path.join(&path);

        if !overwrite && self.vfs.exists(&file_path)? {
            return Err(Error::AlreadyExists(format!(
                "{} already exists",
                file_path
            )));
        }

        self.vfs.write_reader(&file_path, source)?;

        Ok(())
    }
}
