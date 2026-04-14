// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{
    env::{PutProjectError, ReadEnvironment, WriteEnvironment},
    project::{ProjectMut, ProjectRead},
};
use std::{
    collections::{HashMap, hash_map::Entry},
    fmt::Debug,
};

use thiserror::Error;

#[derive(Debug)]
pub struct MemoryStorageEnvironment<Project: Clone> {
    pub projects: HashMap<String, HashMap<String, Project>>,
}

impl<Project: Clone> Default for MemoryStorageEnvironment<Project> {
    fn default() -> Self {
        Self {
            projects: HashMap::default(),
        }
    }
}

impl<Project: ProjectRead + Clone> MemoryStorageEnvironment<Project> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn try_from_iter<T: IntoIterator<Item = (String, Project)>>(
        iter: T,
    ) -> Result<Self, TryFromError<Project>> {
        let mut map = HashMap::<String, HashMap<String, Project>>::new();
        for (iri, project) in iter {
            if let Some(version) = project.version().map_err(TryFromError::Read)? {
                map.entry(iri).or_default().insert(version, project);
            } else {
                return Err(TryFromError::MissingVersion(iri));
            }
        }
        Ok(Self { projects: map })
    }
}

#[derive(Error, Debug)]
pub enum TryFromError<Project: ProjectRead> {
    #[error(transparent)]
    Read(Project::Error),
    #[error("missing version for project with IRI `{0}`")]
    MissingVersion(String),
}

/// Try to construct a `MemoryStorageEnvironment` from an array of pairs of IRIs
/// and project storages.
///
/// All projects must have versions.
///
/// # Returns
///
/// - `Ok(env)` where `env` is a `MemoryStorageEnvironment<Project>` with all
///   projects given.
/// - `Err(error)` where `error` is
///   - `TryFromError<Project>::Read` if cannot be read
///   - `TryFromError<Project>::MissingVersion` if version is `None`
///
/// # Example
///
/// ```rust
/// # use sysand_core::commands::init::do_init_memory;
/// # use sysand_core::env::memory::MemoryStorageEnvironment;
/// # use sysand_core::env::ReadEnvironment;
/// # use sysand_core::project::memory::InMemoryProject;
/// let project1 = do_init_memory("First", Some("a"), "0.0.1", None).unwrap();
/// let project2 = do_init_memory("First", None::<&str>, "0.1.0", None).unwrap();
/// let env = MemoryStorageEnvironment::<InMemoryProject>::try_from([
///     ("urn:kpar:first".into(), project1.clone()),
///     ("urn:kpar:first".into(), project2.clone()),
/// ])
/// .unwrap();
///
/// assert_eq!(
///     project1,
///     env.get_project("urn:kpar:first", "0.0.1").unwrap()
/// );
/// assert_eq!(
///     project2,
///     env.get_project("urn:kpar:first", "0.1.0").unwrap()
/// );
/// ```
impl<Project: ProjectRead + Clone, const N: usize> TryFrom<[(String, Project); N]>
    for MemoryStorageEnvironment<Project>
{
    type Error = TryFromError<Project>;

    fn try_from(value: [(String, Project); N]) -> Result<Self, Self::Error> {
        Self::try_from_iter(value)
    }
}

/// Try to construct a `MemoryStorageEnvironment` from a Vec of pairs of IRIs and
/// project storages.
///
/// All projects must have versions.
///
/// # Returns
///
/// - `Ok(env)` where `env` is a `MemoryStorageEnvironment<Project>` with all
///   projects given.
/// - `Err(error)` where `error` is
///   - `TryFromError<Project>::Read` if cannot be read
///   - `TryFromError<Project>::MissingVersion` if version is `None`
///
/// # Example
///
/// ```rust
/// # use sysand_core::commands::init::do_init_memory;
/// # use sysand_core::env::memory::MemoryStorageEnvironment;
/// # use sysand_core::env::ReadEnvironment;
/// # use sysand_core::project::memory::InMemoryProject;
/// let project1 = do_init_memory("First", Some("a"), "0.0.1", None).unwrap();
/// let project2 = do_init_memory("First", None::<&str>, "0.1.0", None).unwrap();
/// let env = MemoryStorageEnvironment::<InMemoryProject>::try_from(vec![
///     ("urn:kpar:first".into(), project1.clone()),
///     ("urn:kpar:first".into(), project2.clone()),
/// ])
/// .unwrap();
///
/// assert_eq!(
///     project1,
///     env.get_project("urn:kpar:first", "0.0.1").unwrap()
/// );
/// assert_eq!(
///     project2,
///     env.get_project("urn:kpar:first", "0.1.0").unwrap()
/// );
/// ```
impl<Project: ProjectRead + Clone> TryFrom<Vec<(String, Project)>>
    for MemoryStorageEnvironment<Project>
{
    type Error = TryFromError<Project>;

    fn try_from(value: Vec<(String, Project)>) -> Result<Self, Self::Error> {
        Self::try_from_iter(value)
    }
}

impl<Project: ProjectRead + Clone> FromIterator<(String, String, Project)>
    for MemoryStorageEnvironment<Project>
{
    fn from_iter<T: IntoIterator<Item = (String, String, Project)>>(iter: T) -> Self {
        let mut map = HashMap::<String, HashMap<String, Project>>::new();
        for (iri, version, project) in iter {
            map.entry(iri).or_default().insert(version, project);
        }
        Self { projects: map }
    }
}

/// Construct a `MemoryStorageEnvironment` from an array of triples of IRIs, versions
/// and project storages.
///
/// All projects must have versions.
///
/// # Returns
///
/// A `MemoryStorageEnvironment<Project>` with all projects given.
///
/// # Example
///
/// ```rust
/// # use sysand_core::commands::init::do_init_memory;
/// # use sysand_core::env::memory::MemoryStorageEnvironment;
/// # use sysand_core::env::ReadEnvironment;
/// # use sysand_core::project::memory::InMemoryProject;
/// let version1 = "0.0.1".to_string();
/// let version2 = "0.1.0".to_string();
/// let project1 = do_init_memory("First", Some("a"), &version1, None).unwrap();
/// let project2 = do_init_memory("First", None::<&str>, &version2, None).unwrap();
/// let env = MemoryStorageEnvironment::<InMemoryProject>::from([
///     ("urn:kpar:first".into(), version1.clone(), project1.clone()),
///     ("urn:kpar:first".into(), version2.clone(), project2.clone()),
/// ]);
///
/// assert_eq!(
///     project1,
///     env.get_project("urn:kpar:first", version1).unwrap()
/// );
/// assert_eq!(
///     project2,
///     env.get_project("urn:kpar:first", version2).unwrap()
/// );
/// ```
impl<Project: ProjectRead + Clone, const N: usize> From<[(String, String, Project); N]>
    for MemoryStorageEnvironment<Project>
{
    fn from(value: [(String, String, Project); N]) -> Self {
        Self::from_iter(value)
    }
}

/// Construct a `MemoryStorageEnvironment` from Vec of triples of IRIs, versions and
/// project storages.
///
/// All projects must have versions.
///
/// # Returns
///
/// A `MemoryStorageEnvironment<Project>` with all projects given.
///
/// # Example
///
/// ```rust
/// # use sysand_core::commands::init::do_init_memory;
/// # use sysand_core::env::memory::MemoryStorageEnvironment;
/// # use sysand_core::env::ReadEnvironment;
/// # use sysand_core::project::memory::InMemoryProject;
/// let version1 = "0.0.1".to_string();
/// let version2 = "0.1.0".to_string();
/// let project1 = do_init_memory("First", Some("a"), &version1, None).unwrap();
/// let project2 = do_init_memory("First", None::<&str>, &version2, None).unwrap();
/// let env = MemoryStorageEnvironment::<InMemoryProject>::from(vec![
///     ("urn:kpar:first".into(), version1.clone(), project1.clone()),
///     ("urn:kpar:first".into(), version2.clone(), project2.clone()),
/// ]);
///
/// assert_eq!(
///     project1,
///     env.get_project("urn:kpar:first", version1).unwrap()
/// );
/// assert_eq!(
///     project2,
///     env.get_project("urn:kpar:first", version2).unwrap()
/// );
/// ```
impl<Project: ProjectRead + Clone> From<Vec<(String, String, Project)>>
    for MemoryStorageEnvironment<Project>
{
    fn from(value: Vec<(String, String, Project)>) -> Self {
        Self::from_iter(value)
    }
}

// Placeholder for now
#[derive(Error, Debug)]
pub enum MemoryWriteError {}

impl<Project: ProjectMut + Clone + Default> WriteEnvironment for MemoryStorageEnvironment<Project> {
    type WriteError = MemoryWriteError;

    type InterchangeProjectMut = Project;

    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, super::PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>,
    {
        let mut tentative_project = Project::default();

        write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;

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
                if occupied_entry.get().is_empty() {
                    self.projects.remove(uri.as_ref());
                }
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
    #[error("missing project with IRI `{0}`")]
    MissingProject(String),
    #[error("missing project with IRI `{0}` version `{1}`")]
    MissingVersion(String, String),
}

impl<Project: ProjectRead + Clone + Debug> ReadEnvironment for MemoryStorageEnvironment<Project> {
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
            .ok_or_else(|| MemoryReadError::MissingProject(uri.as_ref().to_string()))?
            .keys()
            .map(|x| Ok(x.to_owned()))
            .collect();

        Ok(version_vec)
    }

    type InterchangeProjectRead = Project;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        Ok(self
            .projects
            .get(uri.as_ref())
            .ok_or_else(|| MemoryReadError::MissingProject(uri.as_ref().to_string()))?
            .get(version.as_ref())
            .ok_or_else(|| {
                MemoryReadError::MissingVersion(
                    uri.as_ref().to_string(),
                    version.as_ref().to_string(),
                )
            })?
            .clone())
    }
}

#[cfg(test)]
#[path = "./memory_tests.rs"]
mod tests;
