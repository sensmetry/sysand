// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    env::utils::{CloneError, clone_project},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectMut, ProjectRead},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    io::Read,
};

use thiserror::Error;
use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

/// Project stored in a local directory
#[derive(Clone, Default, Debug)]
pub struct InMemoryProject {
    pub info: Option<InterchangeProjectInfoRaw>,
    pub meta: Option<InterchangeProjectMetadataRaw>,
    pub files: HashMap<Utf8UnixPathBuf, String>,
    pub nominal_sources: Vec<crate::lock::Source>,
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
            files: HashMap::default(),
            nominal_sources: vec![],
        }
    }

    pub fn from_project<Pr: ProjectRead>(
        from: &Pr,
    ) -> Result<InMemoryProject, CloneError<<Pr as ProjectRead>::Error, InMemoryError>> {
        let mut to = InMemoryProject {
            info: None,
            meta: None,
            files: std::collections::HashMap::new(),
            nominal_sources: vec![],
        };

        clone_project(from, &mut to, true)?;

        Ok(to)
    }
}

impl ProjectMut for InMemoryProject {
    fn put_info(
        &mut self,
        info: &InterchangeProjectInfoRaw,
        overwrite: bool,
    ) -> Result<(), InMemoryError> {
        if !overwrite && self.info.is_some() {
            return Err(InMemoryError::AlreadyExists(
                "project already has an information file".to_string(),
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
        if !overwrite && self.meta.is_some() {
            return Err(InMemoryError::AlreadyExists(
                "project already has a meta manifest".to_string(),
            ));
        }

        self.meta = Some(meta.clone());

        Ok(())
    }

    fn write_source<P: AsRef<Utf8UnixPath>, R: Read>(
        &mut self,
        path: P,
        source: &mut R,
        overwrite: bool,
    ) -> Result<(), InMemoryError> {
        let file_entry = self.files.entry(path.as_ref().to_owned());

        if let Entry::Occupied(_) = file_entry {
            if !overwrite {
                return Err(InMemoryError::AlreadyExists(format!(
                    "{} already exists",
                    path.as_ref()
                )));
            }
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
    #[error("project read error: file '{0}' not found")]
    FileNotFound(Box<str>),
    #[error("failed to read from reader: {0}")]
    IoRead(#[from] std::io::Error),
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
        Ok((self.info.clone(), self.meta.clone()))
    }

    type SourceReader<'a> = &'a [u8];

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, InMemoryError> {
        let contents = self
            .files
            .get(&path.as_ref().to_path_buf())
            .ok_or_else(|| InMemoryError::FileNotFound(path.as_ref().as_str().into()))?;

        Ok(contents.as_bytes())
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![]
    }
}
