// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
use std::path::Path;

use serde_json::json;
use thiserror::Error;

pub const DEFAULT_LOCKFILE_NAME: &str = "SysandLock.toml";

#[cfg(feature = "filesystem")]
use crate::project::{editable::EditableProject, local_src::LocalSrcProject};
use crate::{
    lock::{Lock, Project},
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{CanonicalisationError, ProjectRead},
    resolve::ResolveRead,
    solve::pubgrub::{SolverError, solve},
};

#[derive(Error, Debug)]
pub enum LockProjectError<
    PI: ProjectRead,
    PD: ProjectRead,
    R: ResolveRead + std::fmt::Debug + 'static,
> {
    #[error("{0}")]
    InputProjectError(PI::Error),
    #[error("{0}")]
    InputProjectCanonicalisationError(CanonicalisationError<PI::Error>),
    #[error(transparent)]
    LockError(#[from] LockError<PD, R>),
}

#[derive(Error, Debug)]
pub enum LockError<PD: ProjectRead, R: ResolveRead + std::fmt::Debug + 'static> {
    #[error("{0}")]
    DependencyProjectError(PD::Error),
    #[error("{0}")]
    DependencyProjectCanonicalisationError(CanonicalisationError<PD::Error>),
    #[error("{0}")]
    IOError(std::io::Error),
    #[error("incomplete project {0}")]
    IncompleteInputProjectError(String),
    #[error("{0}")]
    ValidationError(InterchangeProjectValidationError),
    #[error("{0}")]
    SolverError(SolverError<R>),
}

pub struct LockOutcome<PI, PD> {
    pub lock: Lock,
    pub inputs: Vec<PI>,
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
    PI: ProjectRead + std::fmt::Debug,
    PD: ProjectRead + std::fmt::Debug,
    I: IntoIterator<Item = PI>,
    R: ResolveRead<ProjectStorage = PD> + std::fmt::Debug,
>(
    projects: I, // TODO: Should this be an iterable over Q?
    resolver: R,
) -> Result<LockOutcome<PI, PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];
    let mut inputs = vec![];

    for project in projects.into_iter() {
        let info = project
            .get_info()
            .map_err(LockProjectError::InputProjectError)?
            .ok_or(LockError::IncompleteInputProjectError(format!(
                "{:?}",
                project
            )))?;

        let canonical_hash = project
            .checksum_canonical_hex()
            .map_err(LockProjectError::InputProjectCanonicalisationError)?
            .ok_or(LockError::IncompleteInputProjectError(format!(
                "{:?}",
                project
            )))?;

        let info_json = json!({
            "name": info.name,
            "version": info.version,
            "usage": info.usage
        });

        lock.project.push(Project {
            info: Some(info_json),
            meta: None,
            iris: vec![],
            checksum: canonical_hash,
            specification: None,
            sources: project.sources(),
        });

        let usages: Result<Vec<InterchangeProjectUsage>, InterchangeProjectValidationError> =
            info.usage.iter().map(|p| p.validate()).collect();
        let usages = usages.map_err(LockError::ValidationError)?;

        inputs.push(project);

        all_deps.extend(usages);
    }

    let LockOutcome {
        lock,
        inputs: _,
        dependencies,
    } = do_lock_extend(lock, all_deps, resolver)?;

    Ok(LockOutcome {
        lock,
        inputs,
        dependencies,
    })
}

/// Solves for compatible set of dependencies based on usages and adds the solution
/// to existing lockfile.
/// Note: The content of the lockfile is not taken into account when solving.
// TODO: Fix this or find a better way.
pub fn do_lock_extend<
    PD: ProjectRead + std::fmt::Debug,
    I: IntoIterator<Item = InterchangeProjectUsage>,
    R: ResolveRead<ProjectStorage = PD> + std::fmt::Debug,
>(
    mut lock: Lock,
    usages: I,
    resolver: R,
) -> Result<LockOutcome<InterchangeProjectUsage, PD>, LockError<PD, R>> {
    let inputs: Vec<_> = usages.into_iter().collect();
    let mut dependencies = vec![];
    let solution = solve(inputs.to_vec(), resolver).map_err(LockError::SolverError)?;

    for (iri, (info, _meta, project)) in solution {
        let info_json = json!({
            "name": info.name,
            "version": info.version,
            "usage": info.usage
        });

        let canonical_hash = project
            .checksum_canonical_hex()
            .map_err(LockError::DependencyProjectCanonicalisationError)?
            .ok_or(LockError::IncompleteInputProjectError(format!(
                "{:?}",
                project
            )))?;

        lock.project.push(Project {
            info: Some(info_json),
            meta: None,
            iris: vec![iri.to_string()],
            checksum: canonical_hash,
            specification: None,
            sources: project.sources(),
        });

        dependencies.push((iri, project));
    }

    Ok(LockOutcome {
        lock,
        inputs,
        dependencies,
    })
}

#[cfg(feature = "filesystem")]
pub type EditableLocalSrcProject = EditableProject<LocalSrcProject>;

/// Treats a project at `path` as an editable project and solves for its dependencies.
#[cfg(feature = "filesystem")]
pub fn do_lock_local_editable<
    P: AsRef<Path>,
    PD: ProjectRead + std::fmt::Debug,
    R: ResolveRead<ProjectStorage = PD> + std::fmt::Debug,
>(
    path: P,
    resolver: R,
) -> Result<
    LockOutcome<EditableLocalSrcProject, PD>,
    LockProjectError<EditableLocalSrcProject, PD, R>,
> {
    let project = EditableProject::new(
        path.as_ref()
            .to_str()
            .ok_or(LockError::IncompleteInputProjectError(
                "project path is not storable".to_string(),
            ))?,
        LocalSrcProject {
            project_path: path.as_ref().canonicalize().map_err(LockError::IOError)?,
        },
    );

    do_lock_projects(std::iter::once(project), resolver)
}
