// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::io::{Cursor, Read};
use sysand_core::context::ProjectContext;

use sysand_core::lock::Source;
use sysand_core::project::{CanonicalizationError, ProjectChecksum};
use sysand_core::{
    manifest::{
        MODEL_TOML_FILE,
        diagnostics::Report,
        kpar_view::{self, ViewError},
    },
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, META_JSON_FILE,
        PROJECT_JSON_FILE, PartialManifest,
    },
    project::{ProjectFormat, ProjectMut, ProjectRead, utils::FsIoError},
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
    #[error("project is missing `model.toml`, or `.project.json` and/or `.meta.json`")]
    MissingInfoMeta,
    /// model-toml.md "Mixed manifests": a project that holds both kinds
    /// of manifest is neither a non-zipped KPAR nor a development
    /// project, and there is no way to tell which was meant.
    #[error(
        "`{0}` has both `model.toml` and a json manifest;\n\
        {SP:>8} it is unclear whether it is a non-zipped KPAR or a development project",
        SP = ' '
    )]
    AmbiguousFormat(Utf8UnixPathBuf),
    /// A command that writes a json manifest was pointed at a
    /// `model.toml` project; writing one would leave the store holding
    /// both, which nothing can read.
    #[error("this is a `model.toml` project; `{0}` cannot be written into one")]
    JsonWriteToTomlProject(&'static str),
    /// The mirror: `sysand migrate` is the way across, and it replaces
    /// both files rather than adding one.
    #[error(
        "this is a `.project.json` project; `model.toml` cannot be written into one \
         (see `sysand migrate`)"
    )]
    TomlWriteToJsonProject,
    /// A source file listed in `sources` could not be indexed.
    #[error(transparent)]
    Symbols(#[from] sysand_core::symbols::SymbolsError),
    #[error("`{0}` is not a valid manifest:\n{1}")]
    InvalidManifest(&'static str, Report),
    /// A `model.toml` that is not one, or does not define a project, or
    /// does not validate. Carried whole, as the filesystem store carries
    /// it.
    #[error(transparent)]
    InvalidModelToml(sysand_core::project::utils::ProjectDeserializationError),
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
    ///
    /// Everything but the storage is `kpar_view`'s, which is free of
    /// Cargo features exactly so that this store answers with the pair a
    /// directory would.
    fn read_model_toml(
        &self,
    ) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), Error> {
        let text = self.vfs.read_string(self.root_path.join(MODEL_TOML_FILE))?;
        kpar_view::of_model_toml(&text, |source| {
            self.vfs.read_string(self.root_path.join(source))
        })
        .map_err(|error| match error {
            ViewError::Manifest(error) => Error::InvalidModelToml(error),
            ViewError::Read(error) => error.into(),
            ViewError::Symbols(error) => Error::Symbols(error),
        })
    }
}

impl ProjectRead for ProjectLocalBrowserStorage {
    type Error = Error;

    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        let toml = self.vfs.exists(self.root_path.join(MODEL_TOML_FILE))?;
        let json = self.vfs.exists(self.root_path.join(PROJECT_JSON_FILE))?
            || self.vfs.exists(self.root_path.join(META_JSON_FILE))?;
        Ok(match (toml, json) {
            (true, true) => ProjectFormat::Both,
            (true, false) => ProjectFormat::Toml,
            (false, true) => ProjectFormat::Json,
            (false, false) => ProjectFormat::Neither,
        })
    }

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self.format()? {
            ProjectFormat::Both => {
                return Err(Error::AmbiguousFormat(self.root_path.clone()));
            }
            ProjectFormat::Toml => {
                let (info, meta) = self.read_model_toml()?;
                return Ok((Some(info), Some(meta)));
            }
            ProjectFormat::Json | ProjectFormat::Neither => {}
        }

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

impl ProjectLocalBrowserStorage {
    /// The same rule the other two stores keep: a json document cannot
    /// be written into a `model.toml` project.
    fn refuse_json_write(&self, file: &'static str) -> Result<(), Error> {
        if self.format()? == ProjectFormat::Toml {
            return Err(Error::JsonWriteToTomlProject(file));
        }
        Ok(())
    }
}

impl ProjectMut for ProjectLocalBrowserStorage {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), Self::Error> {
        self.refuse_json_write(".project.json")?;
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
        self.refuse_json_write(".meta.json")?;
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

    fn put_manifest_document(
        &mut self,
        document: &sysand_core::manifest::edit::DocumentMut,
        overwrite: bool,
    ) -> Result<(), sysand_core::project::ManifestWriteError<Self::Error>> {
        use sysand_core::project::ManifestWriteError;

        let format = self.format().map_err(ManifestWriteError::Project)?;
        // The mirror of `refuse_json_write`, and as narrow: writing into
        // a store that already holds both changes nothing about why it
        // is unreadable.
        if format == ProjectFormat::Json {
            return Err(ManifestWriteError::Project(Error::TomlWriteToJsonProject));
        }

        let manifest_path = self.root_path.join(MODEL_TOML_FILE);
        let exists = self
            .vfs
            .exists(&manifest_path)
            .map_err(|e| ManifestWriteError::Project(Error::from(e)))?;
        if !overwrite && exists {
            return Err(ManifestWriteError::Project(Error::AlreadyExists(
                MODEL_TOML_FILE.into(),
            )));
        }

        self.vfs
            .write_string(&manifest_path, document.to_string())
            .map_err(|e| ManifestWriteError::Project(Error::from(e)))?;
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
