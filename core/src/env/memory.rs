// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    env::{PutProjectError, ReadEnvironment, WriteEnvironment},
    project::memory::InMemoryProject,
};
use std::collections::{HashMap, hash_map::Entry};

use thiserror::Error;

/// Project stored in a local directory
#[derive(Clone, Default, Debug)]
pub struct MemoryStorageEnvironment {
    pub projects: HashMap<String, HashMap<String, InMemoryProject>>,
}

impl MemoryStorageEnvironment {
    pub fn new() -> Self {
        Self::default()
    }
}

// Placeholder for now
#[derive(Error, Debug)]
pub enum MemoryWriteError {}

impl WriteEnvironment for MemoryStorageEnvironment {
    type WriteError = MemoryWriteError;

    type InterchangeProjectMut = InMemoryProject;

    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, super::PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>,
    {
        let mut tentative_project = InMemoryProject::default();

        write_project(&mut tentative_project).map_err(PutProjectError::CallbackError)?;

        self.projects
            .entry(uri.as_ref().to_string())
            .or_default()
            .insert(version.as_ref().to_string(), tentative_project.clone());

        // TODO: Maybe we should not be returning this (so as to avoid clone)?
        Ok(tentative_project)
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        match &mut self.projects.entry(uri.as_ref().to_string()) {
            Entry::Occupied(occupied_entry) => {
                occupied_entry.get_mut().remove(version.as_ref());
                Ok(())
            }
            Entry::Vacant(_) => Ok(()),
        }
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        self.projects.remove(uri.as_ref());
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum MemoryReadError {
    #[error("project read error: {0}")]
    ReadError(String),
    #[error("missing IRI")]
    MissingIRIError(String),
    #[error("missing version")]
    MissingVersionError(String),
}

impl ReadEnvironment for MemoryStorageEnvironment {
    type ReadError = MemoryReadError;

    type UriIter = Vec<Result<String, MemoryReadError>>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let uri_vec: Vec<Result<String, MemoryReadError>> =
            self.projects.keys().map(|x| Ok(x.to_owned())).collect();

        Ok(uri_vec)
    }

    type VersionIter = Vec<Result<String, MemoryReadError>>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let version_vec: Vec<Result<String, MemoryReadError>> = self
            .projects
            .get(uri.as_ref())
            .ok_or_else(|| MemoryReadError::MissingIRIError(uri.as_ref().to_string()))?
            .keys()
            .map(|x| Ok(x.to_owned()))
            .collect();

        Ok(version_vec)
    }

    type InterchangeProjectRead = InMemoryProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(self
            .projects
            .get(uri.as_ref())
            .ok_or_else(|| MemoryReadError::MissingIRIError(uri.as_ref().to_string()))?
            .get(version.as_ref())
            .ok_or_else(|| MemoryReadError::MissingVersionError(version.as_ref().to_string()))?
            .clone())
    }
}
