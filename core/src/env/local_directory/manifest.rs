// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, fmt::Display, num::TryFromIntError};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::{
    commands::sources::{LocalSourcesError, do_sources_local_src_project_no_deps},
    env::local_directory::{LocalDirectoryEnvironment, LocalReadError},
    lock::{Lock, ResolutionError, Source, multiline_list},
    project::{
        local_src::LocalSrcProject,
        utils::{FsIoError, wrapfs},
    },
};

#[derive(Debug, Error)]
pub enum ResolvedManifestError {
    #[error(transparent)]
    ResolutionError(#[from] ResolutionError<LocalReadError>),
    #[error("too many dependencies, unable to convert to i64: {0}")]
    TooManyDependencies(TryFromIntError),
    #[error(transparent)]
    LocalSources(#[from] LocalSourcesError),
    #[error(transparent)]
    Canonicalization(#[from] Box<FsIoError>),
}

impl Lock {
    pub fn to_resolved_manifest<P: AsRef<Utf8Path>>(
        &self,
        env: &LocalDirectoryEnvironment,
        root_path: P,
    ) -> Result<ResolvedManifest, ResolvedManifestError> {
        let resolved_projects = self.resolve_projects(env)?;

        let indices = resolved_projects
            .iter()
            .map(|(p, _)| p)
            .enumerate()
            .flat_map(|(num, p)| p.identifiers.iter().map(move |iri| (iri.clone(), num)))
            .map(|(iri, num)| i64::try_from(num).map(|num| (iri, num)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(ResolvedManifestError::TooManyDependencies)?;
        let indices = HashMap::<String, i64>::from_iter(indices);

        let mut projects = vec![];
        for (project, storage) in resolved_projects {
            let usages = project
                .usages
                .iter()
                .filter_map(|usage| indices.get(&usage.resource))
                .copied()
                .collect();
            let purl = project.get_package_url();
            let publisher = purl
                .as_ref()
                .and_then(|p| p.namespace().map(|ns| ns.to_owned()));
            let name = purl.as_ref().map(|p| p.name().to_owned()).or(project.name);

            if let Some(storage) = storage {
                let directory = storage.root_path();
                projects.push(ResolvedProject {
                    publisher,
                    name,
                    location: ResolvedLocation::Directory(directory),
                    usages,
                });
            } else if let [Source::Editable { editable }, ..] = project.sources.as_slice() {
                let project_path = root_path.as_ref().join(editable.as_str());
                let editable_project = LocalSrcProject {
                    project_path: wrapfs::canonicalize(project_path)?,
                };
                let files = do_sources_local_src_project_no_deps(&editable_project, true)?
                    .into_iter()
                    .collect();
                projects.push(ResolvedProject {
                    publisher,
                    name,
                    location: ResolvedLocation::Files(files),
                    usages,
                });
            }
        }

        Ok(ResolvedManifest { projects })
    }
}

#[derive(Debug)]
pub struct ResolvedManifest {
    pub projects: Vec<ResolvedProject>,
}

impl Display for ResolvedManifest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_toml())
    }
}

impl ResolvedManifest {
    pub fn to_toml(&self) -> DocumentMut {
        let mut doc = DocumentMut::new();
        let mut projects = ArrayOfTables::new();
        for project in &self.projects {
            projects.push(project.to_toml());
        }
        doc.insert("project", Item::ArrayOfTables(projects));

        doc
    }
}

#[derive(Debug)]
pub enum ResolvedLocation {
    Directory(Utf8PathBuf),
    Files(Vec<Utf8PathBuf>),
}

#[derive(Debug)]
pub struct ResolvedProject {
    pub publisher: Option<String>,
    pub name: Option<String>,
    pub location: ResolvedLocation,
    pub usages: Vec<i64>,
}

impl ResolvedProject {
    pub fn to_toml(&self) -> Table {
        let mut table = Table::new();
        if let Some(publisher) = &self.publisher {
            table.insert("publisher", value(publisher));
        }
        if let Some(name) = &self.name {
            table.insert("name", value(name));
        }
        match &self.location {
            ResolvedLocation::Directory(dir) => {
                table.insert("directory", value(dir.as_str()));
            }
            ResolvedLocation::Files(files) => {
                if !files.is_empty() {
                    table.insert(
                        "files",
                        value(multiline_list(
                            files.iter().map(|f| Value::from(f.as_str())),
                        )),
                    );
                }
            }
        }
        if !self.usages.is_empty() {
            let usages = Array::from_iter(self.usages.iter().copied().map(Value::from));
            table.insert("usages", value(usages));
        }
        table
    }
}
