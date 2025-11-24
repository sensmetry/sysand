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

#[derive(Clone, Debug)]
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
    #[error("missing version for project with IRI '{0}'")]
    MissingVersion(String),
}

impl<Project: ProjectRead + Clone, const N: usize> TryFrom<[(String, Project); N]>
    for MemoryStorageEnvironment<Project>
{
    type Error = TryFromError<Project>;

    fn try_from(value: [(String, Project); N]) -> Result<Self, Self::Error> {
        Self::try_from_iter(value)
    }
}

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

impl<Project: ProjectRead + Clone, const N: usize> From<[(String, String, Project); N]>
    for MemoryStorageEnvironment<Project>
{
    fn from(value: [(String, String, Project); N]) -> Self {
        Self::from_iter(value)
    }
}

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
mod test {
    use std::collections::HashMap;

    use crate::{
        env::{
            ReadEnvironment, WriteEnvironment,
            memory::MemoryStorageEnvironment,
            utils::{CloneError, clone_project},
        },
        init::do_init_memory,
        project::memory::{InMemoryError, InMemoryProject},
    };

    #[test]
    fn write_environment() {
        let uri1 = "urn:kpar:first".to_string();
        let uri2 = "urn:kpar:second".to_string();
        let version = "0.0.1".to_string();
        let project1 = do_init_memory("First".to_string(), version.clone(), None).unwrap();
        let project2 = do_init_memory("Second".to_string(), version.clone(), None).unwrap();
        let mut env = MemoryStorageEnvironment::<InMemoryProject>::new();

        env.put_project(&uri1, &version, |p| {
            clone_project(&project1, p, true)?;

            Ok::<(), CloneError<InMemoryError, InMemoryError>>(())
        })
        .unwrap();

        assert_eq!(env.projects.len(), 1);
        assert_eq!(
            &project1,
            env.projects.get(&uri1).unwrap().get(&version).unwrap()
        );

        env.put_project(&uri2, &version, |p| {
            clone_project(&project2, p, true)?;

            Ok::<(), CloneError<InMemoryError, InMemoryError>>(())
        })
        .unwrap();

        assert_eq!(env.projects.len(), 2);
        assert_eq!(
            &project2,
            env.projects.get(&uri2).unwrap().get(&version).unwrap()
        );

        env.del_project_version(&uri1, version).unwrap();

        assert_eq!(env.projects.len(), 1);
        assert!(!env.projects.contains_key(&uri1));

        env.del_uri(&uri2).unwrap();

        assert!(env.projects.is_empty());
        assert!(!env.projects.contains_key(&uri2));
    }

    #[test]
    fn readd_environment() {
        let iri = "urn:kpar:first".to_string();
        let version = "0.0.1".to_string();
        let project = do_init_memory("First".to_string(), version.clone(), None).unwrap();
        let env = MemoryStorageEnvironment {
            projects: HashMap::from([(
                iri.clone(),
                HashMap::from([(version.clone(), project.clone())]),
            )]),
        };

        let uris = env.uris().unwrap();
        assert_eq!(
            vec![&iri],
            uris.iter()
                .map(|uri| uri.as_ref().unwrap())
                .collect::<Vec<_>>()
        );

        let versions = env.versions(&iri).unwrap();
        assert_eq!(
            vec![&version],
            versions
                .iter()
                .map(|version| version.as_ref().unwrap())
                .collect::<Vec<_>>()
        );

        let get_project = env.get_project(iri, version).unwrap();
        assert_eq!(project, get_project);
    }

    #[test]
    fn try_from() {
        let project1 = do_init_memory("First 0.0.1".to_string(), "0.0.1".to_string(), None).unwrap();
        let project2 = do_init_memory("First 0.1.0".to_string(), "0.1.0".to_string(), None).unwrap();
        let project3 = do_init_memory("Second".to_string(), "0.0.1".to_string(), None).unwrap();
        let env = MemoryStorageEnvironment::<InMemoryProject>::try_from([
            ("urn:kpar:first".to_string(), project1.clone()),
            ("urn:kpar:first".to_string(), project2.clone()),
            ("urn:kpar:second".to_string(), project3.clone()),
        ])
        .unwrap();

        assert_eq!(
            project1,
            env.get_project("urn:kpar:first", "0.0.1").unwrap()
        );
        assert_eq!(
            project2,
            env.get_project("urn:kpar:first", "0.1.0").unwrap()
        );
        assert_eq!(
            project3,
            env.get_project("urn:kpar:second", "0.0.1").unwrap()
        );
        assert_eq!(env.projects.len(), 2);
        assert_eq!(env.projects.get("urn:kpar:first").unwrap().len(), 2);
    }
}
