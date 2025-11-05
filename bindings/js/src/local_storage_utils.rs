// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Read;

use serde::{/* Deserialize, */ Serialize};

use sysand_core::project::utils::FsIoError;
use thiserror::Error;
use typed_path::Utf8UnixPath;

#[derive(Clone, Debug)]
pub struct LocalStorageVFS {
    pub prefix: String,
    pub local_storage: web_sys::Storage,
}

#[derive(Error, Debug)]
pub enum LocalStorageError {
    #[error("failed to get 'window' object")]
    NoWindow,
    #[error("failed to get 'window.localStorage' object")]
    NoLocalStorage,
    #[error("JS error: {0:?}")]
    Js(wasm_bindgen::JsValue),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("key '{0}' not found in local storage")]
    KeyNotFound(String),
}

impl From<FsIoError> for LocalStorageError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

pub fn get_local_browser_storage<S: AsRef<str>>(
    prefix: S,
) -> Result<LocalStorageVFS, LocalStorageError> {
    Ok(LocalStorageVFS {
        prefix: prefix.as_ref().to_string(),
        local_storage: web_sys::window()
            .ok_or(LocalStorageError::NoWindow)?
            .local_storage()
            .map_err(LocalStorageError::Js)?
            .ok_or(LocalStorageError::NoLocalStorage)?,
    })
}

impl LocalStorageVFS {
    pub fn to_key<P: AsRef<Utf8UnixPath>>(&self, path: P) -> String {
        format!("{}{}", &self.prefix, path.as_ref())
    }

    pub fn exists<P: AsRef<Utf8UnixPath>>(&self, path: P) -> Result<bool, LocalStorageError> {
        Ok(self
            .local_storage
            .get_item(&self.to_key(path))
            .map_err(LocalStorageError::Js)?
            .is_some())
    }

    pub fn write_serialisable<P: AsRef<Utf8UnixPath>, S: Serialize>(
        &self,
        path: P,
        value: S,
    ) -> Result<(), LocalStorageError> {
        self.write_string(path, serde_json::to_string(&value)?)
    }

    pub fn write_string<P: AsRef<Utf8UnixPath>, S: AsRef<str>>(
        &self,
        path: P,
        value: S,
    ) -> Result<(), LocalStorageError> {
        let key = self.to_key(path);
        self.local_storage
            .set_item(&key, value.as_ref())
            .map_err(LocalStorageError::Js)
    }

    pub fn read_string<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<String, LocalStorageError> {
        let key = self.to_key(path);

        self.local_storage
            .get_item(&key)
            .map_err(LocalStorageError::Js)?
            .ok_or(LocalStorageError::KeyNotFound(key))
    }

    pub fn write_reader<P: AsRef<Utf8UnixPath>, R: Read>(
        &self,
        path: P,
        source: &mut R,
    ) -> Result<(), LocalStorageError> {
        let mut value = String::new();
        source
            .read_to_string(&mut value)
            .map_err(FsIoError::ReadFileHandle)?;
        self.write_string(path, value)
    }

    pub fn delete<P: AsRef<Utf8UnixPath>>(&self, path: P) -> Result<(), LocalStorageError> {
        let key = self.to_key(path);
        self.local_storage
            .remove_item(&key)
            .map_err(LocalStorageError::Js)
    }
}
