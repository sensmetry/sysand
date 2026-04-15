// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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
    context::ProjectContext,
    lock::{Lock, Project, Usage, hash_str},
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage,
        InterchangeProjectValidationError,
    },
    project::{CanonicalizationError, ProjectRead, memory::InMemoryProject, utils::FsIoError},
    resolve::ResolveRead,
    solve::pubgrub::{SolverError, solve},
};

/// The per-project trio every lockfile entry needs: the info/meta records
/// the entry describes plus the canonical digest that identifies it. Both
/// `do_lock_projects` (input-project path) and `do_lock_extend` (resolved
/// dependency path) compute this same trio, which is why
/// [`read_lock_entry_parts`] lives here — each caller differs only in
/// which error variants to map into and how to label a project whose
/// `info` hasn't been read yet.
struct LockEntryParts {
    info: InterchangeProjectInfoRaw,
    meta: InterchangeProjectMetadataRaw,
    canonical_digest: String,
}

/// Read `info` / `meta` / `canonical digest` from `project`, mapping each
/// potential failure into the caller's error type via the four closures:
///
/// - `identifier`: produces a human-readable project id for
///   `IncompleteProject` messages. Called with `None` before `info` is
///   known (caller decides a fallback — an IRI, a workspace path, etc.),
///   and with `Some(&info)` afterwards so the message can name the project
///   precisely.
/// - `on_project_err`: lifts `P::Error` into the caller's error (e.g.
///   `LockError::DependencyProject`).
/// - `on_canon_err`: lifts `CanonicalizationError<P::Error>` similarly.
/// - `on_incomplete`: constructs the "missing field" error variant.
fn read_lock_entry_parts<P, E>(
    project: &P,
    identifier: impl Fn(Option<&InterchangeProjectInfoRaw>) -> String,
    on_project_err: impl Fn(P::Error) -> E,
    on_canon_err: impl FnOnce(CanonicalizationError<P::Error>) -> E,
    on_incomplete: impl Fn(String, &'static str) -> E,
) -> Result<LockEntryParts, E>
where
    P: ProjectRead,
{
    let info = project
        .get_info()
        .map_err(&on_project_err)?
        .ok_or_else(|| on_incomplete(identifier(None), "info"))?;

    let meta = project
        .get_meta()
        .map_err(&on_project_err)?
        .ok_or_else(|| on_incomplete(identifier(Some(&info)), "meta"))?;

    let canonical_digest = project
        .checksum_canonical_hex()
        .map_err(on_canon_err)?
        .ok_or_else(|| on_incomplete(identifier(Some(&info)), "canonical digest"))?;

    Ok(LockEntryParts {
        info,
        meta,
        canonical_digest,
    })
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
pub enum LockError<PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    DependencyProject(PD::Error),
    #[error(transparent)]
    DependencyProjectCanonicalization(CanonicalizationError<PD::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("incomplete project {identifier}: missing {field}")]
    IncompleteProject {
        /// Human-readable identifier for the project (an IRI when the project
        /// was resolved for a usage, a name+version when available locally, or
        /// `<unknown>` as a last resort). Deliberately avoids dumping the
        /// whole `project` via `{:?}` — that output is large, full of impl
        /// detail, and almost never actionable.
        identifier: String,
        /// Which required piece was missing: `info`, `meta`, or
        /// `canonical digest`.
        field: &'static str,
    },
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
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];

    for (identifiers, project) in projects {
        // Before `info` is known: prefer a caller-supplied IRI, fall back to
        // `project.name()`, and only last resort to a placeholder. After
        // `info` is available (meta/canonical paths), the name+version is a
        // strictly better label than whatever was guessed upstream.
        let LockEntryParts {
            info,
            meta,
            canonical_digest,
        } = read_lock_entry_parts(
            project,
            |info| match info {
                Some(info) => format!("{} {}", info.name, info.version),
                None => {
                    if let Some(ids) = &identifiers
                        && let Some(iri) = ids.first()
                    {
                        return iri.as_str().to_owned();
                    }
                    match project.name() {
                        Ok(Some(name)) => name,
                        _ => "<unknown input project>".to_owned(),
                    }
                }
            },
            LockProjectError::InputProjectError,
            LockProjectError::InputProjectCanonicalizationError,
            |identifier, field| LockError::IncompleteProject { identifier, field }.into(),
        )?;

        let sources = project
            .sources(ctx)
            .map_err(LockProjectError::InputProjectError)?;
        debug_assert!(!sources.is_empty());

        lock.projects.push(Project {
            name: Some(info.name),
            publisher: info.publisher,
            version: info.version,
            exports: meta.index.into_keys().collect(),
            identifiers: identifiers
                .map(|ids| ids.into_iter().map(|id| id.into_string()).collect())
                .unwrap_or_default(),
            checksum: canonical_digest,
            sources,
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

    let lock_outcome = do_lock_extend(lock, all_deps, resolver, provided_iris, ctx)?;

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
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    ctx: &ProjectContext,
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

    for (iri, project) in solution {
        let iri_str = iri.as_str().to_owned();
        let LockEntryParts {
            info,
            meta,
            canonical_digest,
        } = read_lock_entry_parts(
            &project,
            |_| iri_str.clone(),
            LockError::DependencyProject,
            LockError::DependencyProjectCanonicalization,
            |identifier, field| LockError::IncompleteProject { identifier, field },
        )?;

        let sources = if !provided_iris.contains_key(iri.as_str()) {
            let sources = project.sources(ctx).map_err(LockError::DependencyProject)?;
            debug_assert!(!sources.is_empty());
            sources
        } else {
            Vec::new()
        };

        let lock_project = Project {
            name: Some(info.name),
            publisher: info.publisher,
            version: info.version.to_string(),
            exports: meta.index.into_keys().collect(),
            identifiers: vec![iri.to_string()],
            checksum: canonical_digest,
            sources,
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
            // Two-pass: first validate every export against `lock_symbols`
            // (and against the other exports of this same project) without
            // mutating either, then commit on success. Mutating
            // `lock_symbols` on the failure path would leave the caller's
            // collision table polluted with an index pointing at a project
            // that was never `push`ed, breaking any future operation on the
            // same `LockOutcome` even though `Err` was returned.
            let new_idx = lock.projects.len();
            let mut local_seen = HashSet::new();
            for s in &lock_project.exports {
                let h = hash_str(s);
                if !local_seen.insert(h) {
                    return Err(LockError::NameCollision(
                        NameCollisionError {
                            symbol: s.to_owned(),
                            pr1: lock_project.clone(),
                            pr2: lock_project,
                        }
                        .into(),
                    ));
                }
                if let Some(conflict_idx) = lock_symbols.get(&h) {
                    return Err(LockError::NameCollision(
                        NameCollisionError {
                            symbol: s.to_owned(),
                            pr1: lock.projects[*conflict_idx].clone(),
                            pr2: lock_project,
                        }
                        .into(),
                    ));
                }
            }
            for s in &lock_project.exports {
                lock_symbols.insert(hash_str(s), new_idx);
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
    PR: AsRef<Utf8Path>,
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    path: P,
    project_root: PR,
    identifiers: Option<Vec<Iri<String>>>,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    resolver: R,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let project = EditableProject::new(
        path.to_path_buf(),
        LocalSrcProject {
            nominal_path: Some(path.to_path_buf()),
            project_path: project_root.as_ref().canonicalize_utf8().map_err(|e| {
                LockError::Io(FsIoError::Canonicalize(project_root.as_ref().join(path), e).into())
            })?,
        },
    );

    do_lock_projects([(identifiers, &project)], resolver, provided_iris, ctx)
}

#[cfg(test)]
#[path = "./lock_tests.rs"]
mod tests;
