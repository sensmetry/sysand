// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use sha2::{Digest, Sha256};
use sysand_core::env::{PutProjectError, ReadEnvironment, WriteEnvironment};
use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{
    io::local_storage::ProjectLocalBrowserStorage,
    local_storage_utils::{self, LocalStorageVFS},
};

pub struct LocalBrowserStorageEnvironment {
    pub root_path: Utf8UnixPathBuf,
    pub vfs: LocalStorageVFS,
    //pub crypto: Crypto,
}

pub fn empty_environment_local_storage<S: AsRef<str>, P: AsRef<Utf8UnixPath>>(
    prefix: S,
    root_path: P,
) -> Result<LocalBrowserStorageEnvironment, Error> {
    let result = LocalBrowserStorageEnvironment {
        root_path: root_path.as_ref().to_path_buf(),
        vfs: local_storage_utils::get_local_browser_storage(prefix.as_ref())?,
    };

    result.vfs.write_string(result.entries_path(), "")?;

    Ok(result)
}

pub fn open_environment_local_storage<S: AsRef<str>, P: AsRef<Utf8UnixPath>>(
    prefix: S,
    root_path: P,
) -> Result<LocalBrowserStorageEnvironment, Error> {
    let result = LocalBrowserStorageEnvironment {
        root_path: root_path.as_ref().to_path_buf(),
        vfs: local_storage_utils::get_local_browser_storage(prefix.as_ref())?,
    };

    if !result.vfs.exists(result.entries_path())? {
        return Err(Error::InvalidEnvironment(
            "missing 'entries.txt'".to_string(),
        ));
    }

    Ok(result)
}

pub const DEFAULT_ENV_NAME: &str = "sysand_env";

const ENTRIES_PATH: &str = "entries.txt";
const VERSIONS_PATH: &str = "versions.txt";

impl LocalBrowserStorageEnvironment {
    pub fn segment_uri<S: AsRef<str>>(&self, uri: S) -> String {
        format!("{:x}", Sha256::digest(uri.as_ref().as_bytes()))
    }

    pub fn entries_path(&self) -> Utf8UnixPathBuf {
        self.root_path.join(ENTRIES_PATH)
    }

    pub fn uri_path<S: AsRef<str>>(&self, uri: S) -> Utf8UnixPathBuf {
        self.root_path.join(self.segment_uri(uri))
    }

    pub fn versions_path<S: AsRef<str>>(&self, uri: S) -> Utf8UnixPathBuf {
        self.uri_path(uri).join(VERSIONS_PATH)
    }

    pub fn project_path<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Utf8UnixPathBuf {
        self.uri_path(uri).join(version.as_ref())
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid environment: {0}")]
    InvalidEnvironment(String),
    #[error(transparent)]
    LocalStorage(#[from] local_storage_utils::LocalStorageError),
}

impl ReadEnvironment for LocalBrowserStorageEnvironment {
    type ReadError = Error;

    type UriIter = Vec<Result<String, Error>>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let entries = self.vfs.read_string(self.entries_path())?;

        Ok(entries.lines().map(|s| Ok(s.to_string())).collect())
    }

    type VersionIter = Vec<Result<String, Error>>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let versions = self.vfs.read_string(self.versions_path(uri))?;

        Ok(versions.lines().map(|s| Ok(s.to_string())).collect())
    }

    type InterchangeProjectRead = ProjectLocalBrowserStorage;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(ProjectLocalBrowserStorage {
            vfs: self.vfs.clone(),
            root_path: self.project_path(&uri, &version),
        })
    }
}

impl WriteEnvironment for LocalBrowserStorageEnvironment {
    type WriteError = Error;

    type InterchangeProjectMut = crate::io::local_storage::ProjectLocalBrowserStorage;

    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, sysand_core::env::PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>,
    {
        let mut project = ProjectLocalBrowserStorage {
            vfs: self.vfs.clone(),
            root_path: self.project_path(&uri, &version),
        };

        // TODO: For production JS-version this should be made more robust
        write_project(&mut project).map_err(sysand_core::env::PutProjectError::Callback)?;

        let mut current_versions = self
            .vfs
            .read_string(self.versions_path(&uri))
            .map_err(Error::LocalStorage)
            .map_err(PutProjectError::Write)?;

        let mut found = false;
        for current_version in current_versions.lines() {
            if current_version == version.as_ref() {
                found = true;
            }
        }

        if !found {
            current_versions.push_str(&format!("\n{}", version.as_ref()));
        }

        self.vfs
            .write_string(self.versions_path(&uri), current_versions)
            .map_err(Error::LocalStorage)
            .map_err(PutProjectError::Write)?;

        Ok(project)
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        let current_versions = self
            .vfs
            .read_string(self.versions_path(&uri))
            .map_err(Error::LocalStorage)?;

        let mut kept_versions = String::new();

        let mut found = false;
        for current_version in current_versions.lines() {
            if current_version != version.as_ref() {
                kept_versions.push_str(&format!("\n{}", current_version));
            } else {
                found = true;
            }
        }

        if found {
            // TODO: This should also try to delete the project files themselves!

            self.vfs
                .write_string(self.versions_path(&uri), kept_versions)
                .map_err(Error::LocalStorage)?;
        }

        Ok(())
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        Ok(self.vfs.delete(self.versions_path(&uri))?)
    }
}
