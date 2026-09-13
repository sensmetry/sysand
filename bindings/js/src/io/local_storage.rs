// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::{Cursor, Read};
use sysand_core::context::ProjectContext;

use sysand_core::lock::Source;
use sysand_core::project::{CanonicalizationError, ProjectChecksum};
use sysand_core::{
    manifest::diagnostics::Report,
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE, PartialManifest,
    },
    project::{ProjectMut, ProjectRead, utils::FsIoError},
};

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::local_storage_utils::{LocalStorageError, LocalStorageVFS, get_local_browser_storage};

#[derive(Debug)]
pub struct ProjectLocalBrowserStorage {
    pub root_path: Utf8UnixPathBuf,
    pub vfs: LocalStorageVFS,
    // TODO: enforce that the project matches the checksum if provided
    // before reading; see LocalKparProject for example
    pub expected_checksum: Option<String>,
}

pub fn open_project_local_storage<S: AsRef<str>, P: AsRef<Utf8UnixPath>>(
    prefix: S,
    root_path: P,
) -> Result<ProjectLocalBrowserStorage, Error> {
    Ok(ProjectLocalBrowserStorage {
        root_path: root_path.as_ref().to_path_buf(),
        vfs: get_local_browser_storage(prefix)?,
        expected_checksum: None,
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
    /// String since JsValue is not Send/Sync
    #[error("JS error: {0}")]
    Js(String),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to serialize information to write it to `{0}`: {1}")]
    Serialize(&'static str, serde_json::Error),
    #[error("failed to serialize: {0}")]
    SerializeHandle(#[from] serde_json::Error),
    #[error("key `{0}` not found in local storage")]
    KeyNotFound(String),
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    #[error("`{0}` is not a valid manifest:\n{1}")]
    InvalidManifest(&'static str, Report),
}

impl From<wasm_bindgen::JsValue> for Error {
    fn from(value: wasm_bindgen::JsValue) -> Self {
        Self::Js(format!("{value:?}"))
    }
}

impl From<FsIoError> for Error {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalStorageError> for Error {
    fn from(value: LocalStorageError) -> Self {
        match value {
            LocalStorageError::NoWindow => Self::NoWindow,
            LocalStorageError::NoLocalStorage => Self::NoLocalStorage,
            LocalStorageError::Js(js_value) => Self::Js(js_value),
            LocalStorageError::Io(error) => Self::Io(error),
            LocalStorageError::Serialize(error) => Self::SerializeHandle(error),
            LocalStorageError::KeyNotFound(key) => Self::KeyNotFound(key),
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
        // The metadata is read first because it carries the metamodel,
        // which a message about the other document may need to name.
        let meta_path = self.root_path.join(META_JSON_FILE);
        let meta: Option<InterchangeProjectMetadataRaw> = match self.vfs.read_string(meta_path) {
            Ok(str) => Some(parse_manifest::<InterchangeProjectMetadataPartial>(
                &str, None,
            )?),
            Err(LocalStorageError::KeyNotFound(_)) => None,
            Err(err) => {
                return Err(err.into());
            }
        };

        let info_path = self.root_path.join(PROJECT_JSON_FILE);
        let info = match self.vfs.read_string(info_path) {
            Ok(str) => Some(parse_manifest::<InterchangeProjectInfoPartial>(
                &str,
                meta.as_ref().and_then(|meta| meta.metamodel.as_deref()),
            )?),
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

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let checksum = match &self.expected_checksum {
            Some(c) => c.clone(),
            None => self
                .checksum_canonical_hex()
                .map_err(|e| match e {
                    CanonicalizationError::ProjectRead(e) => e,
                    CanonicalizationError::FileRead(path, error) => {
                        Error::Io(FsIoError::ReadFile(String::from(path).into(), error).into())
                    }
                })?
                .ok_or(Error::MissingInfoMeta)?,
        };
        Ok(vec![sysand_core::lock::Source::LocalSrc {
            src_path: self.root_path.as_str().into(),
            checksum,
        }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A path names one project. The browser-storage environment hands back
        // one of these per installed version, so the same caveat applies as for
        // `LocalDirectoryEnvironment::get_project_storage`.
        false
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.checksum_canonical_hex() {
            Ok(c) => match c {
                Some(c) => Ok(ProjectChecksum::Project(c)),
                None => Err(Error::MissingInfoMeta),
            },
            Err(e) => match e {
                CanonicalizationError::ProjectRead(e) => Err(e),
                CanonicalizationError::FileRead(path, error) => Err(Error::Io(
                    FsIoError::ReadFile(String::from(path).into(), error).into(),
                )),
            },
        }
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        // LocalStorage should not be treated as a filesystem
        None
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

/// One manifest document from local storage, parsed and validated; its
/// warnings are logged against the document they came from.
fn parse_manifest<T: PartialManifest>(
    text: &str,
    metamodel: Option<&str>,
) -> Result<T::Validated, Error> {
    let partial: T = serde_json::from_str(text).map_err(|e| Error::Serialize(T::FILE, e))?;
    match partial.validate_manifest(metamodel) {
        Ok(validated) => {
            for warning in &validated.warnings {
                log::warn!("`{}`: {warning}", T::FILE);
            }
            Ok(validated.value)
        }
        Err(report) => Err(Error::InvalidManifest(T::FILE, report)),
    }
}
