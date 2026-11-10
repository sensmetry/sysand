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
    manifest::{MODEL_TOML_FILE, edit::DocumentMut, kpar_view},
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw,
        META_JSON_FILE, PROJECT_JSON_FILE,
    },
    project::{
        CanonicalizationError, ManifestError, ManifestWriteError, ProjectDefinition, ProjectFormat,
        ProjectMut, ProjectRead,
    },
};

use super::ProjectChecksum;

/// Project held in memory
#[derive(Clone, Eq, Default, Debug, PartialEq)]
pub struct InMemoryProject {
    pub info: Option<InterchangeProjectInfoRaw>,
    pub meta: Option<InterchangeProjectMetadataRaw>,
    /// The project's `model.toml` text, if it has one; parsing it is
    /// [`ProjectRead::get_project`]'s job.
    ///
    /// A project holding this *and* `info`/`meta` is ambiguous, and is
    /// refused when read.
    pub model_toml: Option<String>,
    pub files: HashMap<Utf8UnixPathBuf, String>,
    pub nominal_sources: Vec<Source>,
    /// What [`ProjectRead::source_may_offer_multiple_versions`] answers. An
    /// in-memory project stands in for whatever source it came from, so the
    /// caller decides.
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
    /// Refuse to write a json manifest into a `model.toml` project.
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
    #[error("failed to read from reader")]
    IoRead(#[from] std::io::Error),
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    /// A json project with only one of its two documents.
    #[error("project is missing `{0}`")]
    MissingDocument(&'static str),
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
}

impl ProjectRead for InMemoryProject {
    type Error = InMemoryError;

    fn get_project(&self) -> Result<Option<ProjectDefinition>, InMemoryError> {
        match self.format()? {
            // Refused before either manifest is read.
            ProjectFormat::Both => Err(InMemoryError::AmbiguousFormat),
            ProjectFormat::Toml => {
                let text = self
                    .model_toml
                    .as_ref()
                    .ok_or(InMemoryError::MissingInfoMeta)?;
                Ok(Some(ProjectDefinition::Toml(Box::new(
                    kpar_view::resolved_of_text(MODEL_TOML_FILE, text)?,
                ))))
            }
            ProjectFormat::Json | ProjectFormat::Neither => {
                match (self.info.clone(), self.meta.clone()) {
                    (Some(info), Some(meta)) => Ok(Some(ProjectDefinition::Json { info, meta })),
                    (None, None) => Ok(None),
                    (Some(_), None) => Err(InMemoryError::MissingDocument(META_JSON_FILE)),
                    (None, Some(_)) => Err(InMemoryError::MissingDocument(PROJECT_JSON_FILE)),
                }
            }
        }
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        Ok(self.get_project()?.map(|project| project.version()))
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(self.get_project()?.map(|project| project.usages()))
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
