// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::{Cursor, Read};
use sysand_core::context::ProjectContext;

use sysand_core::lock::Source;
use sysand_core::project::{CanonicalizationError, FormatError, ProjectChecksum};
use sysand_core::{
    manifest::{MODEL_TOML_FILE, edit::DocumentMut, kpar_view, project::ResolvedProject},
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE, PartialManifest,
    },
    project::{
        ManifestError, ManifestWriteError, ProjectDefinition, ProjectFormat, ProjectMut,
        ProjectRead,
        utils::{FsIoError, ProjectDeserializationError, read_json_manifest},
    },
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
    #[error("failed to serialize")]
    SerializeHandle(#[from] serde_json::Error),
    #[error("key `{0}` not found in local storage")]
    KeyNotFound(String),
    #[error("project is missing `model.toml`, or `.project.json` and/or `.meta.json`")]
    MissingInfoMeta,
    #[error("`{root}` {problem}")]
    Format {
        root: Utf8UnixPathBuf,
        problem: FormatError,
    },
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
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

impl ProjectLocalBrowserStorage {
    /// The KPAR view of the `model.toml` under `root_path`.
    fn read_model_toml(&self) -> Result<ResolvedProject, Error> {
        let text = self.vfs.read_string(self.root_path.join(MODEL_TOML_FILE))?;
        Ok(kpar_view::resolved_of_text(MODEL_TOML_FILE, &text)?)
    }

    /// The json manifest document `T`, `None` when the store has no such
    /// key.
    fn read_json<T: PartialManifest>(
        &self,
        metamodel: Option<&str>,
    ) -> Result<Option<T::Validated>, Error> {
        match self.vfs.read_string(self.root_path.join(T::FILE)) {
            Ok(text) => Ok(Some(read_json_manifest::<T>(text.as_bytes(), metamodel)?)),
            Err(LocalStorageError::KeyNotFound(_)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Write `value` as the json manifest `file`.
    fn write_json(
        &self,
        file: &'static str,
        value: &impl serde::Serialize,
        overwrite: bool,
    ) -> Result<(), Error> {
        self.format()?
            .check_json_write(file)
            .map_err(|problem| self.format_error(problem))?;
        let path = self.root_path.join(file);
        if !overwrite && self.vfs.exists(&path)? {
            return Err(Error::AlreadyExists(file.into()));
        }
        self.vfs.write_serialisable(&path, value)?;
        Ok(())
    }

    /// The error of this project breaking the format rules.
    fn format_error(&self, problem: FormatError) -> Error {
        Error::Format {
            root: self.root_path.clone(),
            problem,
        }
    }

    /// The project's hash, which only a json project has.
    fn project_checksum(&self) -> Result<String, Error> {
        if let Some(checksum) = &self.expected_checksum {
            return Ok(checksum.clone());
        }
        if self.format()? == ProjectFormat::Toml {
            return Err(self.format_error(FormatError::TomlHasNoChecksum));
        }
        self.checksum_canonical_hex()
            .map_err(|e| match e {
                CanonicalizationError::ProjectRead(e) => e,
                CanonicalizationError::FileRead(path, error) => {
                    Error::Io(FsIoError::ReadFile(String::from(path).into(), error).into())
                }
            })?
            .ok_or(Error::MissingInfoMeta)
    }
}

impl ProjectRead for ProjectLocalBrowserStorage {
    type Error = Error;

    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        if self.format()? != ProjectFormat::Toml {
            return Err(ManifestError::NoTomlManifest);
        }
        let text = self
            .vfs
            .read_string(self.root_path.join(MODEL_TOML_FILE))
            .map_err(Error::from)?;
        Ok(sysand_core::manifest::edit::validated(MODEL_TOML_FILE, &text).map_err(Error::from)?)
    }

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        let toml = self.vfs.exists(self.root_path.join(MODEL_TOML_FILE))?;
        let json = self.vfs.exists(self.root_path.join(PROJECT_JSON_FILE))?
            || self.vfs.exists(self.root_path.join(META_JSON_FILE))?;
        Ok(ProjectFormat::from_presence(toml, json))
    }

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        match self.format()? {
            ProjectFormat::Both => {
                let json = if self.vfs.exists(self.root_path.join(PROJECT_JSON_FILE))? {
                    PROJECT_JSON_FILE
                } else {
                    META_JSON_FILE
                };
                return Err(self.format_error(FormatError::Ambiguous { json }));
            }
            ProjectFormat::Toml => {
                return Ok(Some(ProjectDefinition::Toml(Box::new(
                    self.read_model_toml()?,
                ))));
            }
            ProjectFormat::Json | ProjectFormat::Neither => {}
        }

        // The metadata is read first because it carries the metamodel,
        // which a message about the other document may need to name.
        let meta = self.read_json::<InterchangeProjectMetadataPartial>(None)?;
        let info = self.read_json::<InterchangeProjectInfoPartial>(
            meta.as_ref().and_then(|meta| meta.metamodel.as_deref()),
        )?;
        ProjectDefinition::from_json(info, meta).map_err(|problem| self.format_error(problem))
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
        let checksum = self.project_checksum()?;
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
        Ok(ProjectChecksum::Project(self.project_checksum()?))
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
        self.write_json(PROJECT_JSON_FILE, info, overwrite)
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        self.write_json(META_JSON_FILE, meta, overwrite)
    }

    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), ManifestWriteError<Self::Error>> {
        self.format()?
            .check_toml_write()
            .map_err(|problem| self.format_error(problem))?;

        let manifest_path = self.root_path.join(MODEL_TOML_FILE);
        if !overwrite && self.vfs.exists(&manifest_path).map_err(Error::from)? {
            return Err(Error::AlreadyExists(MODEL_TOML_FILE.into()).into());
        }

        let text = sysand_core::manifest::edit::checked_text(document).map_err(Error::from)?;
        self.vfs
            .write_string(&manifest_path, text)
            .map_err(Error::from)?;
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
