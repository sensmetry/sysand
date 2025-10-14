// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
use std::path::PathBuf;
use std::{collections::HashMap, fmt::Debug};

use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject, PathError};
use crate::{
    env::ReadEnvironment,
    lock::{Lock, LockResolutionError},
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{ProjectRead, memory::InMemoryProject},
    resolve::{
        env::EnvResolver,
        memory::{AcceptAll, MemoryResolver},
        priority::{PriorityProject, PriorityResolver},
    },
    solve::pubgrub::SolverError,
};

#[derive(Error, Debug)]
pub enum SourcesError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
}

/// Enumerates source files in a project (as Unix-paths relative to the project root).
/// Combine with `find_project_dependencies` or `enumerate_projects_lock` to get source files
/// of project usages (dependencies).
pub fn do_sources_project_no_deps<Pr: ProjectRead>(
    project: &Pr,
    include_index: bool,
) -> Result<Vec<Utf8UnixPathBuf>, SourcesError<Pr::Error>> {
    let Some(meta) = project.get_meta().map_err(SourcesError::Project)? else {
        return Ok(vec![]);
    };

    Ok(meta
        .validate()?
        .source_paths(include_index)
        .iter()
        .cloned()
        .collect())
}

#[cfg(feature = "filesystem")]
#[derive(Error, Debug)]
pub enum LocalSourcesError {
    #[error(transparent)]
    Project(LocalSrcError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error(transparent)]
    Path(#[from] PathError),
}

#[cfg(feature = "filesystem")]
impl From<SourcesError<LocalSrcError>> for LocalSourcesError {
    fn from(value: SourcesError<LocalSrcError>) -> Self {
        match value {
            SourcesError::Project(error) => LocalSourcesError::Project(error),
            SourcesError::Validation(error) => LocalSourcesError::Validation(error),
        }
    }
}

#[cfg(feature = "filesystem")]
/// Enumerates source files in a local project (as real paths in the filesystem).
/// Combine with `find_project_dependencies` or `enumerate_projects_lock` to get source files
/// of project usages (dependencies).
pub fn do_sources_local_src_project_no_deps(
    project: &LocalSrcProject,
    include_index: bool,
) -> Result<Vec<PathBuf>, LocalSourcesError> {
    let unix_srcs = do_sources_project_no_deps(project, include_index)?;

    let srcs: Result<Vec<_>, _> = unix_srcs
        .iter()
        .map(|path| project.get_source_path(path))
        .collect();

    Ok(srcs?)
}

/// Transitively resolve a list of usages (typically the usages of some project)
/// in an environment and enumerate the resolved projects.
///
/// `provided_iris` are assumed to have been satisfied (including their dependencies)
/// but have to match .
pub fn find_project_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<Vec<<Env as ReadEnvironment>::InterchangeProjectRead>, SolverError<EnvResolver<Env>>> {
    let mut memory_projects = HashMap::default();

    for (k, v) in provided_iris {
        memory_projects.insert(fluent_uri::Iri::parse(k.clone()).unwrap(), v.to_vec());
    }

    let wrapped_resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: memory_projects,
        },
        EnvResolver { env },
    );

    let mut wrapped_result = crate::solve::pubgrub::solve(requested, wrapped_resolver).unwrap();

    Ok(wrapped_result
        .drain()
        .filter_map(|(_, (_, _, project))| match project {
            PriorityProject::HigherProject(_) => None,
            PriorityProject::LowerProject(project) => Some(project),
        })
        .collect())
}

/// Finds all (locked) projects from a `Lock` (typically loaded from a lock file)
/// in an provided environment.
pub fn enumerate_projects_lock<Env: ReadEnvironment>(
    lock: &Lock,
    env: &Env,
) -> Result<
    Vec<<Env as ReadEnvironment>::InterchangeProjectRead>,
    LockResolutionError<<Env as ReadEnvironment>::ReadError>,
> {
    lock.resolve_projects(env)
}
