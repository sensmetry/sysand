// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt::Debug;

#[cfg(feature = "filesystem")]
use camino::Utf8Path;
use thiserror::Error;

pub const DEFAULT_LOCKFILE_NAME: &str = "sysand-lock.toml";

#[cfg(feature = "filesystem")]
use crate::project::{editable::EditableProject, local_src::LocalSrcProject, utils::ToPathBuf};
use crate::{
    lock::{Lock, Project, Usage},
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
    I: IntoIterator<Item = &'a PI>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I, // TODO: Should this be an iterable over Q?
    resolver: R,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];

    for project in projects {
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
            exports: meta.index.keys().cloned().collect(),
            identifiers: vec![],
            checksum: canonical_hash,
            sources: project.sources(),
            usages: info.usage.iter().cloned().map(Usage::from).collect(),
        });

        let usages: Result<Vec<InterchangeProjectUsage>, InterchangeProjectValidationError> =
            info.usage.iter().map(|p| p.validate()).collect();
        let usages = usages.map_err(LockError::Validation)?;

        all_deps.extend(usages);
    }

    let LockOutcome { lock, dependencies } = do_lock_extend(lock, all_deps, resolver)?;

    Ok(LockOutcome { lock, dependencies })
}

/// Solves for compatible set of dependencies based on usages and adds the solution
/// to existing lockfile.
/// Note: The content of the lockfile is not taken into account when solving.
// TODO: Fix this or find a better way.
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

    for (iri, (info, meta, project)) in solution {
        let canonical_hash = project
            .checksum_canonical_hex()
            .map_err(LockError::DependencyProjectCanonicalisation)?
            .ok_or_else(|| LockError::IncompleteInputProject(format!("\n{:?}", project)))?;

        lock.projects.push(Project {
            name: Some(info.name),
            version: info.version.to_string(),
            exports: meta.index.keys().cloned().collect(),
            identifiers: vec![iri.to_string()],
            checksum: canonical_hash,
            sources: project.sources(),
            usages: info.usage.iter().cloned().map(Usage::from).collect(),
        });

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
    PR: AsRef<Utf8Path>,
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    path: P,
    project_root: PR,
    resolver: R,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let project = EditableProject::new(
        // TODO: this is incorrect if project is in a subdir of workspace
        ".".into(),
        LocalSrcProject {
            nominal_path: Some(path.to_path_buf()),
            project_path: project_root
                .as_ref()
                .join(path.as_ref())
                .canonicalize_utf8()
                .map_err(|e| {
                    LockError::Io(
                        FsIoError::Canonicalize(
                            project_root.to_path_buf().join(path.as_ref()),
                            e,
                        )
                        .into(),
                    )
                })?,
        },
    );

    do_lock_projects([&project], resolver)
}
