// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

#[cfg(feature = "filesystem")]
use crate::project::local_src::{LocalSrcError, LocalSrcProject, PathError};
use crate::{
    env::ReadEnvironment,
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{ProjectRead, memory::InMemoryProject},
    resolve::{
        ResolveRead,
        env::EnvResolver,
        memory::{AcceptAll, MemoryResolver},
        priority::{PriorityProject, PriorityResolver},
    },
    solve::pubgrub::SolverError,
    stdlib::known_std_libs,
};

/// Selects which dependency sources a sources enumeration should yield. Whether
/// the project's own sources are listed is controlled separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependencies {
    /// No dependency sources.
    None,
    /// Dependency sources, excluding standard libraries.
    Deps,
    /// Dependency sources, including standard libraries.
    DepsStd,
    /// Only standard-library dependency sources.
    Std,
}

#[derive(Error, Debug)]
#[error("invalid dependencies mode `{0}`")]
pub struct DependenciesParseError(String);

impl TryFrom<&str> for Dependencies {
    type Error = DependenciesParseError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "NONE" => Ok(Dependencies::None),
            "DEPS" => Ok(Dependencies::Deps),
            "DEPS_STD" => Ok(Dependencies::DepsStd),
            "STD" => Ok(Dependencies::Std),
            _ => Err(DependenciesParseError(value.to_owned())),
        }
    }
}

impl TryFrom<String> for Dependencies {
    type Error = DependenciesParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

#[derive(Error, Debug)]
pub enum SourcesError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error("project's `.{name}.json` is invalid")]
    Validation {
        name: &'static str,
        source: InterchangeProjectValidationError,
    },
}

/// Enumerates source files in a project (as relative Unix-paths under the project root).
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
        .validate()
        .map_err(|e| SourcesError::Validation {
            name: "meta",
            source: e,
        })?
        .source_paths(include_index)
        .into_iter()
        .collect())
}

#[cfg(feature = "filesystem")]
#[derive(Error, Debug)]
pub enum LocalSourcesError {
    #[error(transparent)]
    Project(LocalSrcError),
    #[error("project's `.{name}.json` is invalid")]
    Validation {
        name: &'static str,
        source: InterchangeProjectValidationError,
    },
    #[error(transparent)]
    Path(#[from] PathError),
}

#[cfg(feature = "filesystem")]
impl From<SourcesError<LocalSrcError>> for LocalSourcesError {
    fn from(value: SourcesError<LocalSrcError>) -> Self {
        match value {
            SourcesError::Project(error) => LocalSourcesError::Project(error),
            SourcesError::Validation { name, source } => {
                LocalSourcesError::Validation { name, source }
            }
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
) -> Result<Vec<Utf8PathBuf>, LocalSourcesError> {
    let unix_sources = do_sources_project_no_deps(project, include_index)?;

    let sources: Result<Vec<_>, _> = unix_sources
        .iter()
        .map(|path| project.get_source_path(path))
        .collect();

    Ok(sources?)
}

/// Transitively resolves a list of usages (typically the usages of some project)
/// in an environment and enumerates the resolved projects together with their IRIs.
///
/// `provided_iris` are assumed to have been satisfied (including their dependencies)
/// but have to match.
#[allow(clippy::type_complexity)]
fn solve_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<
    Vec<(
        fluent_uri::Iri<String>,
        <Env as ReadEnvironment>::InterchangeProjectRead,
    )>,
    SolverError<impl ResolveRead + Debug + use<Env>>,
> {
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

    let mut wrapped_result = crate::solve::pubgrub::solve(requested, wrapped_resolver)?;

    Ok(wrapped_result
        .drain()
        .filter_map(|(iri, project)| match project {
            PriorityProject::HigherProject(_) => None,
            PriorityProject::LowerProject(project) => Some((iri, project)),
        })
        .collect())
}

/// Transitively resolve a list of usages (typically the usages of some project)
/// in an environment and enumerate the resolved projects.
///
/// `provided_iris` are assumed to have been satisfied (including their dependencies)
/// but have to match.
pub fn find_project_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<
    Vec<<Env as ReadEnvironment>::InterchangeProjectRead>,
    SolverError<impl ResolveRead + Debug + use<Env>>,
> {
    Ok(solve_dependencies(requested, env, provided_iris)?
        .into_iter()
        .map(|(_, project)| project)
        .collect())
}

/// Resolves the dependencies of `requested` in `env` and returns the projects
/// selected by `dependencies`.
///
/// Standard libraries are identified via [`known_std_libs`]: [`Dependencies::Deps`]
/// excludes them, [`Dependencies::Std`] keeps only them and [`Dependencies::DepsStd`]
/// keeps everything. Returns an empty list for [`Dependencies::None`].
pub fn resolve_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    dependencies: Dependencies,
) -> Result<
    Vec<<Env as ReadEnvironment>::InterchangeProjectRead>,
    SolverError<impl ResolveRead + Debug + use<Env>>,
> {
    // For `Deps` the standard libraries are treated as already provided so the
    // solver omits them; otherwise everything is resolved and filtered below.
    let provided_iris = match dependencies {
        Dependencies::Deps => known_std_libs(),
        _ => HashMap::default(),
    };

    let resolved = solve_dependencies(requested, env, &provided_iris)?;

    let std_iris: HashSet<fluent_uri::Iri<String>> = known_std_libs()
        .into_keys()
        .map(|iri| fluent_uri::Iri::parse(iri).expect("BUG: invalid std lib IRI"))
        .collect();

    Ok(resolved
        .into_iter()
        .filter(|(iri, _)| match dependencies {
            Dependencies::None => false,
            Dependencies::Std => std_iris.contains(iri),
            Dependencies::Deps => !std_iris.contains(iri),
            Dependencies::DepsStd => true,
        })
        .map(|(_, project)| project)
        .collect())
}
