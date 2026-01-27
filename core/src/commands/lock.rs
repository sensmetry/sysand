// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use fluent_uri::Iri;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};

#[cfg(feature = "filesystem")]
use camino::Utf8Path;
use thiserror::Error;

pub const DEFAULT_LOCKFILE_NAME: &str = "sysand-lock.toml";

#[cfg(feature = "filesystem")]
use crate::project::{editable::EditableProject, local_src::LocalSrcProject, utils::ToPathBuf};
use crate::{
    lock::{Lock, Project, Usage, hash_str},
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{CanonicalisationError, ProjectRead, utils::FsIoError},
    resolve::ResolveRead,
    solve::pubgrub::{SolverError, solve},
};

#[derive(Error, Debug)]
pub enum LockProjectError<PI: ProjectRead, PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    InputProjectError(PI::Error),
    #[error(transparent)]
    InputProjectCanonicalisationError(CanonicalisationError<PI::Error>),
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
pub enum LockError<PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    DependencyProject(PD::Error),
    #[error(transparent)]
    DependencyProjectCanonicalisation(CanonicalisationError<PD::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("incomplete project{0}")]
    IncompleteInputProject(String),
    #[error(transparent)]
    Validation(InterchangeProjectValidationError),
    #[error(transparent)]
    Solver(SolverError<R>),
    #[error(transparent)]
    NameCollision(Box<NameCollisionError>),
}

pub struct LockOutcome<PD> {
    pub lock: Lock,
    pub dependencies: Vec<(fluent_uri::Iri<String>, PD)>,
}

/// Generates a lockfile by solving for a (compatible) set of interchange projects
/// to satisfy interchange project usages in `info`.
///
/// Typically `PI` will be a single `EditableProject` wrapping some local workspace project.
/// See `do_lock_local_editable`.
///
/// `resolver` is used to interpret the usage IRIs.
///
/// Returns a lockfile, as well as a list of dependency projects to install (in addition to)
/// `projects`.
pub fn do_lock_projects<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    I: IntoIterator<Item = (Option<Vec<Iri<String>>>, &'a PI)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];

    for (identifiers, project) in projects {
        let info = project
            .get_info()
            .map_err(LockProjectError::InputProjectError)?
            .ok_or_else(|| LockError::IncompleteInputProject(format!("\n{:?}", project)))?;
        let meta = project
            .get_meta()
            .map_err(LockProjectError::InputProjectError)?
            .ok_or_else(|| LockError::IncompleteInputProject(format!("{:?}", project)))?;

        let canonical_hash = project
            .checksum_canonical_hex()
            .map_err(LockProjectError::InputProjectCanonicalisationError)?
            .ok_or_else(|| LockError::IncompleteInputProject(format!("\n{:?}", project)))?;

        lock.projects.push(Project {
            name: Some(info.name),
            version: info.version,
            exports: meta.index.into_keys().collect(),
            identifiers: identifiers
                .map(|ids| ids.into_iter().map(|id| id.into_string()).collect())
                .unwrap_or_default(),
            checksum: canonical_hash,
            sources: project.sources(),
            usages: info
                .usage
                .iter()
                .map(|u| Usage::from(u.resource.clone()))
                .collect(),
        });

        let usages: Result<Vec<InterchangeProjectUsage>, InterchangeProjectValidationError> =
            info.usage.iter().map(|p| p.validate()).collect();
        let usages = usages.map_err(LockError::Validation)?;

        all_deps.extend(usages);
    }

    let lock_outcome = do_lock_extend(lock, all_deps, resolver)?;

    Ok(lock_outcome)
}

/// Solves for compatible set of dependencies based on usages and adds the solution
/// to existing lockfile.
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
    I: IntoIterator<Item = InterchangeProjectUsage>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    mut lock: Lock,
    usages: I,
    resolver: R,
) -> Result<LockOutcome<PD>, LockError<PD, R>> {
    let inputs: Vec<_> = usages.into_iter().collect();
    let mut dependencies = vec![];
    let solution = solve(inputs, resolver).map_err(LockError::Solver)?;
    let mut lock_projects = HashSet::new();
    let mut lock_symbols = HashMap::new();
    for (i, p) in lock.projects.iter().enumerate() {
        lock_projects.insert(p.hash_val());
        for s in p.exports.iter() {
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

    for (iri, (info, meta, project)) in solution {
        let canonical_hash = project
            .checksum_canonical_hex()
            .map_err(LockError::DependencyProjectCanonicalisation)?
            .ok_or_else(|| LockError::IncompleteInputProject(format!("\n{:?}", project)))?;

        let lock_project = Project {
            name: Some(info.name),
            version: info.version.to_string(),
            exports: meta.index.into_keys().collect(),
            identifiers: vec![iri.to_string()],
            checksum: canonical_hash,
            sources: project.sources(),
            usages: info
                .usage
                .into_iter()
                .map(|u| Usage::from(u.resource))
                .collect(),
        };
        if lock_projects.contains(&lock_project.hash_val()) {
            log::debug!(
                "not adding project `{}` ({}) to lock, as lock already contains it",
                iri,
                lock_project.version
            );
        } else {
            for s in &lock_project.exports {
                if let Some(conflict_idx) = lock_symbols.insert(hash_str(s), lock.projects.len()) {
                    return Err(LockError::NameCollision(
                        NameCollisionError {
                            symbol: s.to_owned(),
                            pr1: if conflict_idx == lock.projects.len() {
                                // Will happen if `lock_project` exports duplicate symbols
                                lock_project.clone()
                            } else {
                                lock.projects[conflict_idx].clone()
                            },
                            pr2: lock_project,
                        }
                        .into(),
                    ));
                }
            }
            lock.projects.push(lock_project);
        }

        dependencies.push((iri, project));
    }

    Ok(LockOutcome { lock, dependencies })
}

#[cfg(feature = "filesystem")]
pub type EditableLocalSrcProject = EditableProject<LocalSrcProject>;

/// Treats a project at `path` as an editable project and solves for its dependencies.
#[cfg(feature = "filesystem")]
pub fn do_lock_local_editable<
    P: AsRef<Utf8Path>,
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    path: P,
    identifiers: Option<Vec<Iri<String>>>,
    resolver: R,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let project = EditableProject::new(
        // TODO: this is incorrect if project is in a subdir of workspace
        ".".into(),
        LocalSrcProject {
            project_path: path.to_path_buf(),
        },
    );

    do_lock_projects([(identifiers, &project)], resolver)
}

#[cfg(test)]
mod tests {
    use crate::{
        commands::lock::{LockError, do_lock_extend},
        lock::{Lock, Project},
        resolve::null::NullResolver,
    };

    #[test]
    fn lock_export_conflict() {
        let exports = vec!["sym1".into(), "sym2".into(), "sym3".into()];

        let lock = Lock {
            lock_version: String::new(),
            projects: vec![
                Project {
                    name: Some("test1".into()),
                    version: String::new(),
                    exports: exports.clone(),
                    identifiers: vec!["test1".into()],
                    checksum: String::new(),
                    sources: vec![],
                    usages: vec![],
                },
                Project {
                    name: Some("test2".into()),
                    version: String::new(),
                    exports,
                    identifiers: vec!["test2".into()],
                    checksum: String::new(),
                    sources: vec![],
                    usages: vec![],
                },
            ],
        };
        let res = do_lock_extend(lock, [], NullResolver {});

        assert!(matches!(res, Err(LockError::NameCollision(_))));
    }
}
