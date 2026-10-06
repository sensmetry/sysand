// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, fmt::Debug};

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

use crate::{
    env::ReadEnvironment,
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{ProjectRead, utils::Identifier},
    resolve::{
        ResolveRead,
        env::EnvResolver,
        memory::{AcceptAll, MemoryResolver},
        priority::{PriorityProject, PriorityResolver},
    },
    solve::pubgrub::SolverError,
    stdlib::known_std_libs,
    utils::ProvidedProjects,
};
#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::{LocalDirectoryEnvironment, LocalReadError},
    model::UsageRef,
    project::local_src::{LocalSrcError, LocalSrcProject, PathError},
};
#[cfg(feature = "filesystem")]
use fluent_uri::Iri;
#[cfg(feature = "filesystem")]
use semver::{Version, VersionReq};

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
            "NONE" => Ok(Self::None),
            "DEPS" => Ok(Self::Deps),
            "DEPS_STD" => Ok(Self::DepsStd),
            "STD" => Ok(Self::Std),
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
            SourcesError::Project(error) => Self::Project(error),
            SourcesError::Validation { name, source } => Self::Validation { name, source },
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
/// `provided_usages` are assumed to have been satisfied (including their dependencies)
/// and are not included in the returned list
fn solve_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    provided_usages: &ProvidedProjects,
) -> Result<
    Vec<(Identifier, <Env as ReadEnvironment>::InterchangeProjectRead)>,
    SolverError<impl ResolveRead + Debug + use<Env>>,
> {
    let resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: provided_usages.to_owned(),
        },
        EnvResolver { env },
    );

    // `base_path` does not matter here, since the resolver only looks in env
    let mut resolved = crate::solve::pubgrub::solve(requested, None, resolver)?;

    Ok(resolved
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
/// `provided_usages` are assumed to have been satisfied (including their dependencies)
/// and are not included in the returned list
pub fn find_project_dependencies<Env: ReadEnvironment + Debug + 'static>(
    requested: Vec<InterchangeProjectUsage>,
    env: Env,
    provided_usages: &ProvidedProjects,
) -> Result<
    Vec<<Env as ReadEnvironment>::InterchangeProjectRead>,
    SolverError<impl ResolveRead + Debug + use<Env>>,
> {
    Ok(solve_dependencies(requested, env, provided_usages)?
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
    // No need to resolve dependencies if they are not gonna be used
    if dependencies == Dependencies::None {
        return Ok(Vec::new());
    }
    let std_libs = known_std_libs();

    // For `Deps` the standard libraries are treated as already provided so the
    // solver omits them; otherwise everything is resolved and filtered below.
    let empty = HashMap::default();
    let provided_iris = match dependencies {
        Dependencies::Deps => &std_libs,
        _ => &empty,
    };

    let resolved = solve_dependencies(requested, env, provided_iris)?;

    Ok(resolved
        .into_iter()
        .filter(|(iri, _)| match dependencies {
            Dependencies::Std => std_libs.contains_key(iri),
            // std_libs are already filtered out by `solve_dependencies`
            Dependencies::Deps | Dependencies::DepsStd => true,
            Dependencies::None => false,
        })
        .map(|(_, project)| project)
        .collect())
}

#[cfg(feature = "filesystem")]
#[derive(Error, Debug)]
pub enum SourcesEnvError {
    #[error("invalid IRI `{0}`")]
    IriParse(Box<str>, #[source] fluent_uri::ParseError),
    #[error("invalid version constraint `{0}`")]
    VersionReqParse(Box<str>, #[source] semver::Error),
    #[error(transparent)]
    EnvRead(#[from] LocalReadError),
    #[error(transparent)]
    ProjectRead(#[from] LocalSrcError),
    #[error("unable to find project `{project}`{} in local environment", version.as_ref().map(|vr| format!(" ({vr})")).unwrap_or_default())]
    NotFound {
        project: Box<str>,
        version: Option<VersionReq>,
    },
    #[error("project `{0}` is missing project information")]
    MissingInfo(Box<str>),
    #[error("project `{project}` has invalid metadata")]
    InvalidMetadata {
        project: Box<str>,
        source: InterchangeProjectValidationError,
    },
    #[error(transparent)]
    Sources(#[from] LocalSourcesError),
    #[error("failed to resolve dependencies of project `{project}`")]
    Dependencies {
        project: Box<str>,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Enumerates source files (as real paths in the filesystem) of `project`
/// installed in `env`, and (depending on `dependencies`) of its dependencies
/// installed in `env`. If `version` is given, the first installed version
/// matching it is used, otherwise the first installed version.
///
/// The project is looked up by the identifier it is installed under: the
/// IRI, or the identifier derived from publisher and name. The latter is
/// a `pkg:sysand` PURL, matched by any spelling that normalizes to it, unless
/// publisher or name are not valid in a PURL; then only the spelling the
/// project declares matches
#[cfg(feature = "filesystem")]
pub fn do_sources_env(
    env: LocalDirectoryEnvironment,
    project: UsageRef<'_>,
    version: Option<&VersionReq>,
    no_own: bool,
    dependencies: Dependencies,
) -> Result<Vec<Utf8PathBuf>, SourcesEnvError> {
    let identifier = match project {
        UsageRef::Resource(iri) => Identifier::from_iri(&iri),
        UsageRef::Typed(publisher, name) => Identifier::from_pub_name(publisher, name),
    };
    let mut candidates = env.candidate_projects(&identifier)?.into_iter();

    let installed = match version {
        // No version constraints, so choose the first candidate
        None => candidates.next(),
        Some(vr) => loop {
            let Some(candidate) = candidates.next() else {
                break None;
            };
            // Projects with non-semver versions cannot match the constraint
            if let Some(info) = candidate.get_info()? {
                match Version::parse(&info.version) {
                    Ok(v) if vr.matches(&v) => break Some(candidate),
                    Ok(_) => {}
                    Err(e) => log::debug!(
                        "ignoring env project `{}` because it has invalid semver version:\n{e}",
                        info.name
                    ),
                }
            }
        },
    };
    let Some(installed) = installed else {
        return Err(SourcesEnvError::NotFound {
            project: project.to_string().into(),
            version: version.cloned(),
        });
    };

    let mut result = vec![];
    if !no_own {
        result.extend(do_sources_local_src_project_no_deps(&installed, true)?);
    }

    if dependencies != Dependencies::None {
        let Some(info) = installed.get_info()? else {
            return Err(SourcesEnvError::MissingInfo(project.to_string().into()));
        };
        let info = info
            .validate()
            .map_err(|source| SourcesEnvError::InvalidMetadata {
                project: project.to_string().into(),
                source,
            })?;
        let deps = resolve_dependencies(info.usage, env, dependencies).map_err(|e| {
            SourcesEnvError::Dependencies {
                project: project.to_string().into(),
                source: Box::new(e),
            }
        })?;
        for dep in deps {
            result.extend(do_sources_local_src_project_no_deps(&dep, true)?);
        }
    }

    Ok(result)
}

/// Same as [`do_sources_env`], but takes an unparsed IRI and version constraint
#[cfg(feature = "filesystem")]
pub fn do_sources_env_parse(
    env: LocalDirectoryEnvironment,
    iri: String,
    version: Option<String>,
    no_own: bool,
    dependencies: Dependencies,
) -> Result<Vec<Utf8PathBuf>, SourcesEnvError> {
    let iri = Iri::parse(iri).map_err(|(e, iri)| SourcesEnvError::IriParse(iri.into(), e))?;
    let version = version
        .map(|vr| {
            VersionReq::parse(&vr).map_err(|e| SourcesEnvError::VersionReqParse(vr.into(), e))
        })
        .transpose()?;
    do_sources_env(
        env,
        UsageRef::Resource(iri.borrow()),
        version.as_ref(),
        no_own,
        dependencies,
    )
}
