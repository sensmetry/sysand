// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{/* Deserialize, */ Serialize};

use thiserror::Error;
use typed_path::Utf8UnixPath;

#[derive(Clone, Debug)]
pub struct LocalStorageVFS {
    pub prefix: String,
    pub local_storage: web_sys::Storage,
}

#[derive(Error, Debug)]
pub enum LocalStorageError {
    // #[error("refusing to overwrite")]
    // AlreadyExists(String),
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
    #[error("de/serialisation error")]
    KeyNotFoundError(),
}

pub fn get_local_browser_storage<S: AsRef<str>>(
    prefix: S,
) -> Result<LocalStorageVFS, LocalStorageError> {
    Ok(LocalStorageVFS {
        prefix: prefix.as_ref().to_string(),
        local_storage: web_sys::window()
            .ok_or(LocalStorageError::NoWindow())?
            .local_storage()
            .map_err(LocalStorageError::JSError)?
            .ok_or(LocalStorageError::NoLocalStorage())?,
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
            .map_err(LocalStorageError::JSError)?
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
            .map_err(LocalStorageError::JSError)
    }

    pub fn read_string<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<String, LocalStorageError> {
        let key = self.to_key(path);

        self.local_storage
            .get_item(&key)
            .map_err(LocalStorageError::JSError)?
            .ok_or(LocalStorageError::KeyNotFoundError())
    }

    pub fn write_reader<P: AsRef<Utf8UnixPath>, R: std::io::Read>(
        &self,
        path: P,
        source: &mut R,
    ) -> Result<(), LocalStorageError> {
        let mut value = String::new();
        source.read_to_string(&mut value)?;
        self.write_string(path, value)
    }

    pub fn delete<P: AsRef<Utf8UnixPath>>(&self, path: P) -> Result<(), LocalStorageError> {
        let key = self.to_key(path);
        self.local_storage
            .remove_item(&key)
            .map_err(LocalStorageError::JSError)
    }
}
