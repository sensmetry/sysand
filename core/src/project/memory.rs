// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, hash_map::Entry},
    io::Read,
};

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

use crate::{
    context::ProjectContext,
    env::utils::{CloneError, clone_project},
    lock::Source,
    manifest::{
        MODEL_TOML_FILE,
        edit::DocumentMut,
        kpar_view::{self, ViewError},
        project::ResolvedProject,
    },
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, PROJECT_JSON_FILE},
    project::{
        CanonicalizationError, ManifestError, ManifestWriteError, ProjectFormat, ProjectMut,
        ProjectRead,
    },
};

use super::ProjectChecksum;

/// Project stored in a local directory
#[derive(Clone, Eq, Default, Debug, PartialEq)]
pub struct InMemoryProject {
    pub info: Option<InterchangeProjectInfoRaw>,
    pub meta: Option<InterchangeProjectMetadataRaw>,
    /// The project's `model.toml`, if it has one. Text rather than a
    /// parse, because that is what a file holds and what
    /// [`ProjectRead::format`] answers from; parsing it is
    /// [`ProjectRead::manifest`]'s job and goes through the same
    /// `kpar_view` path a project on disk uses, so the two cannot
    /// diverge.
    ///
    /// A project holding this *and* `info`/`meta` is the ambiguity
    /// model-toml.md refuses. It is constructible -- every field here is
    /// public -- so it is answered rather than declared impossible.
    pub model_toml: Option<String>,
    pub files: HashMap<Utf8UnixPathBuf, String>,
    pub nominal_sources: Vec<Source>,
    /// What [`ProjectRead::source_may_offer_multiple_versions`] answers. An
    /// in-memory project stands in for whatever source put it there, so this
    /// is left for the caller to decide
    pub source_may_offer_multiple_versions: bool,
}

impl InMemoryProject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_info_meta(
        info: InterchangeProjectInfoRaw,
        meta: InterchangeProjectMetadataRaw,
    ) -> Self {
        Self {
            info: Some(info),
            meta: Some(meta),
            model_toml: None,
            files: HashMap::default(),
            nominal_sources: vec![],
            source_may_offer_multiple_versions: false,
        }
    }

    pub fn from_project<Pr: ProjectRead>(
        from: &Pr,
    ) -> Result<Self, CloneError<<Pr as ProjectRead>::Error, InMemoryError>> {
        let mut to = Self {
            info: None,
            meta: None,
            model_toml: None,
            files: std::collections::HashMap::new(),
            nominal_sources: vec![],
            source_may_offer_multiple_versions: false,
        };

        clone_project(from, &mut to, true)?;

        Ok(to)
    }
}

impl InMemoryProject {
    /// The KPAR view of this project's `model.toml`.
    ///
    /// Rebuilt per call rather than cached: a cached pair would have to
    /// sit in a field, and this type derives `PartialEq`, `Eq` and
    /// `Clone` -- two projects holding the same manifest would then
    /// compare unequal on nothing more than whether either had been
    /// read. `LocalSrcProject` can afford the cache because it derives
    /// none of the three.
    fn view(
        &self,
    ) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), InMemoryError> {
        let text = self
            .model_toml
            .as_ref()
            .ok_or(InMemoryError::MissingInfoMeta)?;
        kpar_view::of_model_toml(text, |source| {
            self.files
                .get(source)
                .cloned()
                .ok_or_else(|| InMemoryError::FileNotFound(source.to_path_buf()))
        })
        .map_err(|error| match error {
            ViewError::Manifest(error) => InMemoryError::Manifest(error),
            ViewError::Read(error) => error,
            ViewError::Symbols(error) => InMemoryError::Symbols(error),
        })
    }

    /// The mirror of `LocalSrcProject::refuse_json_write`.
    fn refuse_json_write(&self, file: &'static str) -> Result<(), InMemoryError> {
        if self.format()? == ProjectFormat::Toml {
            return Err(InMemoryError::JsonWriteToTomlProject(file));
        }
        Ok(())
    }
}

impl ProjectMut for InMemoryProject {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), InMemoryError> {
        self.refuse_json_write(PROJECT_JSON_FILE)?;
        if !overwrite && self.info.is_some() {
            return Err(InMemoryError::AlreadyExists(
                "project already has an information file".to_owned(),
            ));
        }

        self.info = Some(info.clone());

        Ok(())
    }

    fn put_meta(
        &mut self,
        meta: &InterchangeProjectMetadataRaw,
        overwrite: bool,
    ) -> Result<(), InMemoryError> {
        self.refuse_json_write(crate::model::META_JSON_FILE)?;
        if !overwrite && self.meta.is_some() {
            return Err(InMemoryError::AlreadyExists(
                "project already has a meta manifest".to_owned(),
            ));
        }

        self.meta = Some(meta.clone());

        Ok(())
    }

    fn put_manifest_document(
        &mut self,
        document: &DocumentMut,
        overwrite: bool,
    ) -> Result<(), ManifestWriteError<InMemoryError>> {
        if self.format().map_err(ManifestWriteError::Project)? == ProjectFormat::Json {
            return Err(ManifestWriteError::Project(
                InMemoryError::TomlWriteToJsonProject,
            ));
        }
        if !overwrite && self.model_toml.is_some() {
            return Err(ManifestWriteError::Project(InMemoryError::AlreadyExists(
                format!("project already has a `{MODEL_TOML_FILE}`"),
            )));
        }
        self.model_toml = Some(document.to_string());
        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), InMemoryError> {
        let file_entry = self.files.entry(path.as_ref().to_owned());

        if !overwrite && let Entry::Occupied(_) = file_entry {
            return Err(InMemoryError::AlreadyExists(format!(
                "`{}` already exists",
                path.as_ref()
            )));
        }

        let mut buf = String::new();
        source.read_to_string(&mut buf)?;
        file_entry.insert_entry(buf);

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum InMemoryError {
    #[error("{0}")]
    AlreadyExists(String),
    #[error("project read error: file `{0}` not found")]
    FileNotFound(Utf8UnixPathBuf),
    #[error("failed to read from reader: {0}")]
    IoRead(#[from] std::io::Error),
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    /// The project holds a `model.toml` *and* json documents.
    #[error("project has both a `model.toml` and json manifests")]
    AmbiguousFormat,
    #[error("project is a `model.toml` project; `{0}` cannot be written into one")]
    JsonWriteToTomlProject(&'static str),
    #[error(
        "project is a `.project.json` project; `model.toml` cannot be written into one \
         (see `sysand migrate`)"
    )]
    TomlWriteToJsonProject,
    #[error(transparent)]
    Manifest(#[from] crate::project::utils::ProjectDeserializationError),
    #[error(transparent)]
    Symbols(#[from] crate::symbols::SymbolsError),
}

impl ProjectRead for InMemoryProject {
    type Error = InMemoryError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        InMemoryError,
    > {
        match self.format()? {
            // model-toml.md "Mixed manifests": a project holding both
            // kinds is refused before either is read, as it is on disk.
            ProjectFormat::Both => Err(InMemoryError::AmbiguousFormat),
            ProjectFormat::Toml => {
                let (info, meta) = self.view()?;
                Ok((Some(info), Some(meta)))
            }
            ProjectFormat::Json | ProjectFormat::Neither => {
                Ok((self.info.clone(), self.meta.clone()))
            }
        }
    }

    fn format(&self) -> Result<ProjectFormat, InMemoryError> {
        let json = self.info.is_some() || self.meta.is_some();
        Ok(match (self.model_toml.is_some(), json) {
            (true, true) => ProjectFormat::Both,
            (true, false) => ProjectFormat::Toml,
            (false, true) => ProjectFormat::Json,
            (false, false) => ProjectFormat::Neither,
        })
    }

    fn manifest_document(
        &self,
    ) -> Result<crate::manifest::edit::DocumentMut, ManifestError<InMemoryError>> {
        let Some(text) = &self.model_toml else {
            return Err(ManifestError::NoTomlManifest);
        };
        crate::manifest::edit::validated(MODEL_TOML_FILE, text)
            .map_err(|e| ManifestError::Project(InMemoryError::Manifest(e)))
    }

    fn manifest(&self) -> Result<ResolvedProject, ManifestError<InMemoryError>> {
        let Some(text) = &self.model_toml else {
            return Err(ManifestError::NoTomlManifest);
        };
        kpar_view::resolved_of_text(MODEL_TOML_FILE, text)
            .map_err(|error| ManifestError::Project(InMemoryError::Manifest(error)))
    }

    type SourceReader<'a> = &'a [u8];

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, InMemoryError> {
        let path_buf = path.as_ref().to_path_buf();
        let contents = self
            .files
            .get(&path_buf)
            .ok_or(InMemoryError::FileNotFound(path_buf))?;

        Ok(contents.as_bytes())
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        debug_assert_ne!(self.nominal_sources, []);
        Ok(self.nominal_sources.clone())
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        self.source_may_offer_multiple_versions
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        let checksum = self
            .checksum_canonical_hex()
            .map_err(|e| match e {
                CanonicalizationError::ProjectRead(e) => e,
                CanonicalizationError::FileRead(_, _) => unreachable!(),
            })?
            .ok_or(InMemoryError::MissingInfoMeta)?;
        Ok(ProjectChecksum::Project(checksum))
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
}

#[cfg(test)]
#[path = "./memory_tests.rs"]
mod tests;
