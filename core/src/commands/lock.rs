// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use semver::VersionReq;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::{self, Debug},
};
#[cfg(feature = "filesystem")]
use typed_path::Utf8UnixPath;

#[cfg(feature = "filesystem")]
use camino::Utf8Path;
use thiserror::Error;

pub const DEFAULT_LOCKFILE_NAME: &str = "sysand-lock.toml";

#[cfg(feature = "filesystem")]
use crate::project::{editable::EditableProject, local_src::LocalSrcProject, utils::wrapfs};
use crate::{
    context::ProjectContext,
    lock::{Lock, Project, Usage, hash_str},
    model::{IndexUsage, InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{
        CanonicalizationError, ProjectRead,
        utils::{FsIoError, Identifier},
    },
    resolve::ResolveRead,
    solve::pubgrub::{SolveOptions, SolverError, solve},
    utils::ProvidedProjects,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteField {
    Info,
    Meta,
    CanonicalDigest,
}

impl fmt::Display for IncompleteField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("info"),
            Self::Meta => f.write_str("meta"),
            Self::CanonicalDigest => f.write_str("canonical digest"),
        }
    }
}

#[derive(Error, Debug)]
pub enum LockProjectError<PI: ProjectRead, PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    InputProjectError(PI::Error),
    #[error(transparent)]
    InputProjectCanonicalizationError(CanonicalizationError<PI::Error>),
    #[error(transparent)]
    LockError(#[from] LockError<PD, R>),
}

#[derive(Error, Debug)]
#[error(
        "symbol name `{}` is exported more than once in lockfile:\nproject 1:\n{:#}\nproject 2:\n{:#}", .symbol, .pr1.to_toml(), .pr2.to_toml()
    )]
pub struct NameCollisionError {
    pub symbol: String,
    pub pr1: Project,
    pub pr2: Project,
}

#[derive(Error, Debug)]
#[error(
    "symbol name `{}` is exported more than once by the same project:\n{:#}",
    .symbol,
    .project.to_toml(),
)]
pub struct SelfNameCollisionError {
    pub symbol: String,
    pub project: Project,
}

/// The project that declares a usage
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredBy {
    /// One of the projects being locked (or the request itself), labelled
    /// for messages. The user can edit it.
    Input(String),
    /// A dependency, labelled for messages. Its publisher has to fix it.
    Dependency(String),
}

impl fmt::Display for DeclaredBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(label) => f.write_str(label),
            Self::Dependency(label) => write!(f, "dependency {label}"),
        }
    }
}

/// An index usage resolved to a project whose publisher or name is spelled
/// differently from the usage's.
#[derive(Error, Debug)]
#[error(
    "index usage `{usage_publisher}/{usage_name}` in {declared_by} resolved to version \
     {version} of a project that declares itself `{}/{name}`;\n{}",
    .publisher.as_deref().unwrap_or("<none>"),
    match .declared_by {
        DeclaredBy::Input(_) => "spell the usage as the project does",
        DeclaredBy::Dependency(_) =>
            "the usage is not yours to edit: it has to be fixed by that dependency's publisher",
    }
)]
pub struct IndexUsageMismatchError {
    pub usage_publisher: String,
    pub usage_name: String,
    pub declared_by: DeclaredBy,
    pub version: String,
    pub publisher: Option<String>,
    pub name: String,
}

#[derive(Error, Debug)]
pub enum LockError<PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    DependencyProject(PD::Error),
    #[error(transparent)]
    DependencyProjectCanonicalization(CanonicalizationError<PD::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("incomplete project {project_label}: missing {field}")]
    IncompleteProject {
        /// Human-readable label for the project (an IRI when the project
        /// was resolved for a usage, a name+version when available locally, or
        /// `<unknown input project>` as a last resort). Deliberately avoids
        /// dumping the whole `project` via `{:?}` — that output is large,
        /// full of impl detail, and almost never actionable.
        project_label: String,
        /// Which required piece was missing: `info`, `meta`, or
        /// `canonical digest`.
        field: IncompleteField,
    },
    #[error("project `{identifier}` has invalid metadata")]
    InvalidProject {
        identifier: String,
        source: InterchangeProjectValidationError,
    },
    #[error(transparent)]
    Solver(SolverError<R>),
    #[error(transparent)]
    IndexUsageMismatch(Box<IndexUsageMismatchError>),
    #[error(transparent)]
    NameCollision(Box<NameCollisionError>),
    #[error(transparent)]
    SelfNameCollision(Box<SelfNameCollisionError>),
}

#[derive(Debug)]
pub struct LockOutcome<PD: Debug> {
    pub lock: Lock,
    pub dependencies: Vec<(Identifier, PD)>,
}

/// Generates a lockfile by solving for a (compatible) set of interchange projects
/// to satisfy interchange project usages in `info`.
///
/// Typically `PI` will be a single `EditableProject` wrapping some local workspace project.
/// See `do_lock_local_editable`.
///
/// `projects` contains `(identifiers, project)` pairs. If `identifiers` is `Some()`,
/// it must not be an empty list
///
/// `resolver` is used to interpret the usage IRIs.
///
/// Returns a lockfile, as well as a list of dependency projects to install (in addition to)
/// `projects`.
pub fn do_lock_projects<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    // TODO: maybe take Vec<Iri<String>>, since empty vec is tolerated
    I: IntoIterator<Item = (Option<Vec<Iri<String>>>, &'a PI)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
    options: SolveOptions,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];

    for (identifiers, project) in projects {
        let input_project_label = || match identifiers.as_ref().and_then(|ids| ids.first()) {
            Some(iri) => format!("`{iri}`"),
            None => "<unknown input project>".to_owned(),
        };

        let info = project
            .get_info()
            .map_err(LockProjectError::InputProjectError)?
            .ok_or_else(|| {
                LockProjectError::LockError(LockError::IncompleteProject {
                    project_label: input_project_label(),
                    field: IncompleteField::Info,
                })
            })?;
        let named_project_label = format!("`{}` {}", info.name, info.version);
        let validated_info = info.validate().map_err(|e| LockError::InvalidProject {
            identifier: named_project_label.clone(),
            source: e,
        })?;
        let meta = project
            .get_meta()
            .map_err(LockProjectError::InputProjectError)?
            .ok_or_else(|| {
                LockProjectError::LockError(LockError::IncompleteProject {
                    project_label: named_project_label.clone(),
                    field: IncompleteField::Meta,
                })
            })?;
        let sources = project
            .sources(ctx)
            .map_err(LockProjectError::InputProjectError)?;
        debug_assert_ne!(sources, []);

        lock.projects.push(Project {
            name: info.name,
            publisher: info.publisher,
            version: info.version,
            exports: meta.index.into_keys().collect(),
            identifiers: identifiers
                .map(|ids| ids.into_iter().map(Into::into).collect())
                .unwrap_or_default(),
            sources,
            usages: validated_info.usage.iter().map(Usage::from).collect(),
        });

        all_deps.extend(
            validated_info
                .usage
                .into_iter()
                .map(|usage| (usage, DeclaredBy::Input(named_project_label.clone()))),
        );
    }

    let lock_outcome = do_lock_extend(lock, all_deps, resolver, options, provided_usages, ctx)?;

    Ok(lock_outcome)
}

/// Solves for compatible set of dependencies based on usages and adds the solution
/// to existing lockfile. Each usage comes with the project that declares it,
/// for error messages.
/// Note: The content of the lockfile is taken into account only to avoid
///       including duplicate projects (same project, same version) in lock.
///       This can cause incorrect version selection, as possible
///       constraints from current usages are not taken into account
// TODO: Take into account existing lock when solving deps:
//       - to account for all constraints
//       - to not waste time looking up deps that are
//         already in lockfile
pub fn do_lock_extend<
    PD: ProjectRead + Debug,
    I: IntoIterator<Item = (InterchangeProjectUsage, DeclaredBy)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    mut lock: Lock,
    usages: I,
    resolver: R,
    options: SolveOptions,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockError<PD, R>> {
    let (inputs, declared_by): (Vec<_>, Vec<_>) = usages.into_iter().unzip();
    // Index usages, to check against the projects they resolve to
    let mut index_usages: Vec<(IndexUsage<VersionReq>, DeclaredBy)> = inputs
        .iter()
        .zip(declared_by)
        .filter_map(|(usage, declared_by)| match usage {
            InterchangeProjectUsage::Index(index) => Some((index.clone(), declared_by)),
            _ => None,
        })
        .collect();
    // Publisher, name and version of each solved project
    let mut solved = HashMap::new();
    let mut dependencies = vec![];
    #[cfg(feature = "filesystem")]
    let base_path = ctx.workspace_or_project_root();
    #[cfg(not(feature = "filesystem"))]
    let base_path = None;
    let solution = solve(inputs, base_path, resolver, options).map_err(LockError::Solver)?;
    let mut lock_projects = HashSet::new();
    let mut lock_symbols = HashMap::new();
    for (i, p) in lock.projects.iter().enumerate() {
        if let Some(id) = p.identifiers.first() {
            // FIXME: better deduplication. What to consider? Previously this was
            // done based on canonical checksum, but such rigor is not necessary,
            // since if symbols of any two projects overlap the lock will fail anyway.
            // Current way may produce a lockfile that does not satisfy all version
            // constraints.
            lock_projects.insert(id.clone());
        }
        for s in &p.exports {
            if let Some(conflict_idx) = lock_symbols.insert(hash_str(s), i) {
                return Err(LockError::NameCollision(
                    NameCollisionError {
                        symbol: s.to_owned(),
                        pr1: lock.projects[conflict_idx].clone(),
                        pr2: p.clone(),
                    }
                    .into(),
                ));
            }
        }
    }

    for (identifier, project) in solution {
        // TODO: use get_info, that can be more efficient
        let info = project
            .get_info()
            .map_err(LockError::DependencyProject)?
            .ok_or_else(|| LockError::IncompleteProject {
                project_label: identifier.to_string(),
                field: IncompleteField::Info,
            })?;
        let validated_info = info.validate().map_err(|e| LockError::InvalidProject {
            identifier: identifier.to_string(),
            source: e,
        })?;
        let meta = project
            .get_meta()
            .map_err(LockError::DependencyProject)?
            .ok_or_else(|| LockError::IncompleteProject {
                project_label: identifier.to_string(),
                field: IncompleteField::Meta,
            })?;

        for usage in &validated_info.usage {
            if let InterchangeProjectUsage::Index(index) = usage {
                index_usages.push((
                    index.clone(),
                    DeclaredBy::Dependency(format!(
                        "`{}` {} (`{identifier}`)",
                        info.name, info.version
                    )),
                ));
            }
        }
        solved.insert(
            identifier.clone(),
            (
                info.publisher.clone(),
                info.name.clone(),
                info.version.clone(),
            ),
        );

        let sources = if provided_usages.contains_key(&identifier) {
            Vec::new()
        } else {
            let sources = project.sources(ctx).map_err(LockError::DependencyProject)?;
            debug_assert_ne!(sources, []);
            sources
        };

        let lock_project = Project {
            name: info.name,
            publisher: info.publisher,
            version: info.version.clone(),
            exports: meta.index.into_keys().collect(),
            identifiers: vec![identifier.to_string()],
            sources,
            // TODO: into_iter
            usages: validated_info.usage.iter().map(Usage::from).collect(),
        };
        if lock_projects.contains(identifier.as_str()) {
            log::debug!(
                "not adding project `{identifier}` ({}) to lock, as lock already contains it",
                lock_project.version
            );
        } else {
            let new_idx = lock.projects.len();
            for s in &lock_project.exports {
                let h = hash_str(s);
                match lock_symbols.entry(h) {
                    Entry::Occupied(occupied) => {
                        let conflict_idx = *occupied.get();
                        if conflict_idx == new_idx {
                            return Err(LockError::SelfNameCollision(
                                SelfNameCollisionError {
                                    symbol: s.to_owned(),
                                    project: lock_project,
                                }
                                .into(),
                            ));
                        }
                        return Err(LockError::NameCollision(
                            NameCollisionError {
                                symbol: s.to_owned(),
                                pr1: lock.projects[conflict_idx].clone(),
                                pr2: lock_project,
                            }
                            .into(),
                        ));
                    }
                    Entry::Vacant(vacant) => {
                        vacant.insert(new_idx);
                    }
                }
            }
            lock.projects.push(lock_project);
        }

        dependencies.push((identifier, project));
    }

    check_index_usages(index_usages, &solved).map_err(LockError::IndexUsageMismatch)?;

    Ok(LockOutcome { lock, dependencies })
}

/// Check that each index usage's publisher and name are spelled exactly as
/// the project it resolved to (in `solved`, as publisher, name and version)
/// spells them. Normalization makes them resolve to the same project
/// regardless, so this is the only place a misspelling is caught.
fn check_index_usages(
    index_usages: Vec<(IndexUsage<VersionReq>, DeclaredBy)>,
    solved: &HashMap<Identifier, (Option<String>, String, String)>,
) -> Result<(), Box<IndexUsageMismatchError>> {
    for (usage, declared_by) in index_usages {
        let identifier = Identifier::from_pub_name(&usage.publisher, &usage.name);
        // Not being in the solution is not a mismatch: e.g. a project the
        // caller provides, or one already in the lock
        let Some((publisher, name, version)) = solved.get(&identifier) else {
            continue;
        };
        if publisher.as_deref() != Some(usage.publisher.as_str()) || *name != usage.name {
            return Err(Box::new(IndexUsageMismatchError {
                usage_publisher: usage.publisher,
                usage_name: usage.name,
                declared_by,
                version: version.clone(),
                publisher: publisher.clone(),
                name: name.clone(),
            }));
        }
    }
    Ok(())
}

#[cfg(feature = "filesystem")]
pub type EditableLocalSrcProject = EditableProject<LocalSrcProject>;

/// Treats a project at `path` as an editable project and solves for its dependencies.
#[cfg(feature = "filesystem")]
pub fn do_lock_local_editable<
    P: AsRef<Utf8UnixPath>,
    PR: AsRef<Utf8Path>,
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    path: P,
    project_root: PR,
    identifiers: Option<Vec<Iri<String>>>,
    provided_usages: &ProvidedProjects,
    resolver: R,
    options: SolveOptions,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let path = path.as_ref();
    let project = EditableProject::new(
        path.to_owned(),
        LocalSrcProject::new_access(
            wrapfs::canonicalize(&project_root).map_err(LockError::Io)?,
            None,
        ),
    );

    do_lock_projects(
        [(identifiers, &project)],
        resolver,
        options,
        provided_usages,
        ctx,
    )
}

#[cfg(test)]
#[path = "./lock_tests.rs"]
mod tests;
