// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
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
    model::{InterchangeProjectUsage, InterchangeProjectValidationError},
    project::{
        CanonicalizationError, ProjectRead,
        utils::{FsIoError, Identifier},
    },
    resolve::{ResolveRead, UsageOrigin},
    solve::pubgrub::{SolverError, solve_with_bases},
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
    /// A member inherits a group usage that names a local path.
    ///
    /// `kpar_view::info` drops each usage's base, so such a usage would
    /// reach the solver as a bare relative path and be joined onto the
    /// member's directory -- while a group entry's paths are relative to
    /// the workspace root. Refused rather than silently misresolved.
    #[error(
        "member `{member}` inherits the usage `{usage}`, whose path `{path}` is relative to the \n\
        {SP:>8} workspace root; a workspace lock cannot express that yet",
        SP = ' '
    )]
    InheritedLocalUsage {
        member: String,
        usage: String,
        path: String,
    },
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

/// A typed usage resolved to a project whose publisher or name is spelled
/// differently from the usage's.
#[derive(Error, Debug)]
pub struct TypedUsageMismatchError {
    /// The usage's kind, with an article (e.g. "an index")
    pub kind: &'static str,
    pub usage_publisher: String,
    pub usage_name: String,
    pub declared_by: DeclaredBy,
    pub version: String,
    pub publisher: Option<String>,
    pub name: String,
}

impl fmt::Display for TypedUsageMismatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            kind,
            usage_publisher,
            usage_name,
            declared_by,
            version,
            publisher,
            name,
        } = self;
        let usage = format!("{kind} usage `{usage_publisher}/{usage_name}` in {declared_by}");
        match publisher {
            Some(publisher) => writeln!(
                f,
                "{usage} resolved to version {version} of `{publisher}/{name}`, but is rejected \
                 because its spelling does not match the project's;"
            )?,
            // No spelling can name it
            None => writeln!(
                f,
                "{usage} resolved to version {version} of `{name}`, which declares no \
                 publisher, so no typed usage can name it;"
            )?,
        }
        match (declared_by, publisher) {
            (DeclaredBy::Input(_), Some(publisher)) => {
                write!(f, "spell the usage exactly as `{publisher}/{name}`")
            }
            (DeclaredBy::Input(_), None) => {
                f.write_str("use a resource usage of it instead (e.g. `sysand add --iri <IRI>`)")
            }
            (DeclaredBy::Dependency(_), _) => f.write_str(
                "the usage is not yours to edit: it has to be fixed by that dependency's publisher",
            ),
        }
    }
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
    /// The metamodel direction rule, over the graph the solve produced.
    ///
    /// `design/model-toml.md` runs the edge checks "once before the
    /// lockfile is written and again when the lockfile is validated on
    /// `sync` and `build`"; this is the first of the two.
    ///
    /// Carries the direction error rather than `lock::ValidationError`:
    /// every message of that type ends "in lockfile", and at this point
    /// there is no lockfile yet.
    #[error(transparent)]
    MetamodelDirection(Box<crate::graph::MetamodelDirectionError<String>>),
    #[error("project {identifier} has invalid metadata")]
    InvalidProject {
        /// Human-readable label for the project, already quoted
        identifier: String,
        source: InterchangeProjectValidationError,
    },
    #[error(transparent)]
    Solver(SolverError<R>),
    #[error(transparent)]
    TypedUsageMismatch(Box<TypedUsageMismatchError>),
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
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    #[cfg(feature = "filesystem")]
    let base_path = ctx.project_root();
    #[cfg(not(feature = "filesystem"))]
    let base_path = None;
    lock_projects(
        projects
            .into_iter()
            .map(|(identifiers, project)| (identifiers, project, base_path.clone())),
        resolver,
        provided_usages,
        ctx,
        None,
    )
}

/// [`do_lock_projects`], with each input project's own base path.
///
/// A workspace's members are its input projects, and a member's `dir`
/// usages are relative to that member -- so one base for all of them is
/// wrong as soon as two members sit at different depths. See
/// [`do_lock_extend_with_bases`] for what `ctx` is still for.
pub fn do_lock_projects_with_bases<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    I: IntoIterator<
        Item = (
            Option<Vec<Iri<String>>>,
            &'a PI,
            Option<camino::Utf8PathBuf>,
        ),
    >,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    lock_projects(projects, resolver, provided_usages, ctx, None)
}

/// [`do_lock_projects_with_bases`], with the spelling of the typed usage
/// of `respelled` that the projects declare not checked (see
/// [`check_typed_usages`])
fn lock_projects<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    I: IntoIterator<
        Item = (
            Option<Vec<Iri<String>>>,
            &'a PI,
            Option<camino::Utf8PathBuf>,
        ),
    >,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<LockOutcome<PD>, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();

    let mut all_deps = vec![];

    for (identifiers, project, base_path) in projects {
        let input_project_label = || match identifiers.as_ref().and_then(|ids| ids.first()) {
            Some(iri) => format!("`{iri}`"),
            None => "<unknown input project>".to_owned(),
        };

        // Which project declared the usages below, for the conflicts a
        // failed solve reports. A workspace lock passes each member's PURL
        // as its `identifiers` (`do_lock_workspace`), which is what makes
        // "`a` requires ^1 and `b` requires ^2" sayable. An ordinary
        // single-project lock passes `None` (`do_lock_local_editable`), so
        // its conflicts stay anonymous -- naming a lone project to the person who just ran `lock` in it says
        // nothing. Taken before `identifiers` is moved below.
        let declarer = identifiers
            .as_ref()
            .and_then(|ids| ids.first())
            .map(ToString::to_string);

        let origin = UsageOrigin::of_format(
            project
                .format()
                .map_err(LockProjectError::InputProjectError)?,
        );

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
            // Both construction sites record it: without the root's own
            // metamodel the edge checks read it as an OMG project and a
            // Syside root with a Syside dependency is a false direction
            // error.
            metamodel: meta.metamodel.clone(),
            exports: meta.index.into_keys().collect(),
            identifiers: identifiers
                .map(|ids| ids.into_iter().map(Into::into).collect())
                .unwrap_or_default(),
            sources,
            usages: validated_info.usage.iter().map(Usage::from).collect(),
        });

        // What a toml `kpar-url` usage declared, by IRI. The view this
        // loop iterates cannot carry it -- `InterchangeProjectUsage::Resource`
        // is KerML's untyped usage, where the IRI *is* the identity --
        // so it is read from the manifest here and travels on the
        // `ResolutionInfo`, which is what lets `FileResolver` check the
        // target the same way it checks a `dir` or `kpar-path` one.
        //
        // Read here, where the usages are resolved, so that the check
        // applies to whatever resolves through this function rather than
        // to `lock` alone.
        //
        // A project with no `model.toml` answers `Err(NoTomlManifest)`,
        // which is not an error: a json project declares no `kpar-url`
        // usages at all.
        let declared_identities: HashMap<String, (String, String)> = project
            .manifest()
            .map(|manifest| {
                manifest
                    .usages
                    .iter()
                    .filter_map(|located| match &located.usage {
                        crate::manifest::usage::Usage::RemoteKpar {
                            publisher: Some(publisher),
                            name,
                            url,
                        } => Some((url.to_string(), (publisher.to_string(), name.to_string()))),
                        // A standard-library usage declares no
                        // publisher, so there is nothing to check it
                        // against.
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        all_deps.extend(validated_info.usage.into_iter().map(|usage| {
            let declared_identity = match &usage {
                crate::model::InterchangeProjectUsage::Resource { resource, .. } => {
                    declared_identities.get(&resource.to_string()).cloned()
                }
                _ => None,
            };
            (
                crate::resolve::ResolutionInfo::new_declared_in(usage, base_path.clone(), origin)
                    .declared_by(declarer.clone())
                    .declaring_identity(declared_identity),
                DeclaredBy::Input(named_project_label.clone()),
            )
        }));
    }

    let lock_outcome = lock_extend(lock, all_deps, resolver, provided_usages, ctx, respelled)?;

    Ok(lock_outcome)
}

/// Solves for compatible set of dependencies based on usages and adds the solution
/// to existing lockfile. Each usage comes with the project that declares it,
/// for error messages, and with the manifest it was declared in; a caller
/// that did not read one passes [`UsageOrigin::Json`].
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
    I: IntoIterator<Item = (InterchangeProjectUsage, DeclaredBy, UsageOrigin)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    lock: Lock,
    usages: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockError<PD, R>> {
    #[cfg(feature = "filesystem")]
    let base_path = ctx.project_root();
    #[cfg(not(feature = "filesystem"))]
    let base_path = None;
    lock_extend(
        lock,
        usages.into_iter().map(|(usage, declared_by, origin)| {
            (
                crate::resolve::ResolutionInfo::new_declared_in(usage, base_path.clone(), origin),
                declared_by,
            )
        }),
        resolver,
        provided_usages,
        ctx,
        None,
    )
}

/// [`do_lock_extend`], with each usage's own base path.
///
/// A workspace is solved as one unit and its members do not share a
/// base: `workspaces.md` "Intra-workspace usages" gives a usage's paths
/// as "`dir`, relative to the member, or a group entry relative to the
/// root".
///
/// `ctx` still supplies the root a solved dependency's sources are
/// relativized against, which is a *different* thing from the base its
/// usages resolve against. A `ctx` with neither a project nor a workspace
/// makes `LocalSrcProject::sources` panic, so a caller passing bases
/// must still give one.
pub fn do_lock_extend_with_bases<
    PD: ProjectRead + Debug,
    I: IntoIterator<Item = (crate::resolve::ResolutionInfo, DeclaredBy)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    lock: Lock,
    usages: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockError<PD, R>> {
    lock_extend(lock, usages, resolver, provided_usages, ctx, None)
}

/// [`do_lock_extend_with_bases`], with the spelling of the typed usage of
/// `respelled` among the inputs not checked (see [`check_typed_usages`])
fn lock_extend<
    PD: ProjectRead + Debug,
    I: IntoIterator<Item = (crate::resolve::ResolutionInfo, DeclaredBy)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    mut lock: Lock,
    usages: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<LockOutcome<PD>, LockError<PD, R>> {
    let (inputs, declared_by): (Vec<_>, Vec<_>) = usages.into_iter().unzip();
    // Typed usages, to check against the projects they resolve to
    let mut typed_usages: Vec<(TypedSpelling, DeclaredBy)> = inputs
        .iter()
        .zip(declared_by)
        .filter_map(|(info, declared_by)| Some((TypedSpelling::of_info(info)?, declared_by)))
        .collect();
    // Publisher, name and version of each solved project
    let mut solved = HashMap::new();
    let mut dependencies = vec![];
    let solution = solve_with_bases(inputs, resolver).map_err(LockError::Solver)?;
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
            identifier: format!("`{identifier}` {}", info.version),
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
            if let Some(spelling) = TypedSpelling::of(usage) {
                typed_usages.push((
                    spelling,
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
            metamodel: meta.metamodel.clone(),
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

    check_typed_usages(typed_usages, &solved, respelled).map_err(LockError::TypedUsageMismatch)?;

    // "once before the lockfile is written" (`design/model-toml.md`,
    // Metamodel). Here rather than in the callers so that no caller can
    // skip it, and rather than a full
    // `Lock::validate` because a lock at this point legitimately fails
    // the other rules: an input project may carry no identifier, and
    // `sysand env install --path` records a source outside the project
    // root. Release mismatches are warnings, reported here rather than by
    // a lockfile's validation, which runs on every read.
    for mismatch in lock
        .check_metamodel_edges()
        .map_err(LockError::MetamodelDirection)?
    {
        log::warn!("{mismatch}");
    }

    Ok(LockOutcome { lock, dependencies })
}

/// A typed usage's kind, publisher and name
struct TypedSpelling {
    /// With an article, see
    /// [`crate::model::InterchangeProjectUsageG::kind_with_article`]
    kind: &'static str,
    publisher: String,
    name: String,
    /// What the usage resolves under: the publisher and name for a typed
    /// usage, the IRI for a `kpar-url` usage that declares them
    identifier: Identifier,
}

impl TypedSpelling {
    /// `None` for a resource usage, which names no publisher or name
    fn of(usage: &InterchangeProjectUsage) -> Option<Self> {
        let (publisher, name) = usage.typed_publisher_name()?;
        Some(Self {
            kind: usage.kind_with_article(),
            publisher: publisher.to_owned(),
            name: name.to_owned(),
            identifier: Identifier::from_pub_name(publisher, name),
        })
    }

    /// [`Self::of`], and also a toml `kpar-url` usage that declares a
    /// publisher and name: the view cannot carry them (the resource usage
    /// is KerML's untyped one, where the IRI is the identity), so they
    /// travel on the [`crate::resolve::ResolutionInfo`] instead, read off
    /// the manifest in [`do_lock_projects_with_bases`]. The target is then
    /// checked exactly as a `dir` or `kpar-path` target is.
    fn of_info(info: &crate::resolve::ResolutionInfo) -> Option<Self> {
        if let Some(spelling) = Self::of(info.usage()) {
            return Some(spelling);
        }
        let (publisher, name) = info.declared_identity()?;
        Some(Self {
            kind: "a `kpar-url`",
            publisher: publisher.to_owned(),
            name: name.to_owned(),
            identifier: info.id(),
        })
    }
}

/// Check that each typed usage's publisher and name are spelled exactly as
/// the project it resolved to (in `solved`, as publisher, name and version)
/// spells them. Normalization makes them resolve to the same project
/// regardless, so this is the only place a misspelling is caught.
///
/// The typed usage of `respelled` that the projects being locked declare is
/// not checked: its caller takes the spelling from the project it resolved to
/// (see `sysand add`). Dependencies' usages of it are checked as any other.
fn check_typed_usages(
    typed_usages: Vec<(TypedSpelling, DeclaredBy)>,
    solved: &HashMap<Identifier, (Option<String>, String, String)>,
    respelled: Option<&Identifier>,
) -> Result<(), Box<TypedUsageMismatchError>> {
    for (usage, declared_by) in typed_usages {
        if matches!(declared_by, DeclaredBy::Input(_)) && respelled == Some(&usage.identifier) {
            continue;
        }
        // Not being in the solution is not a mismatch: e.g. a project the
        // caller provides, or one already in the lock
        let Some((publisher, name, version)) = solved.get(&usage.identifier) else {
            continue;
        };
        if publisher.as_deref() != Some(usage.publisher.as_str()) || *name != usage.name {
            return Err(Box::new(TypedUsageMismatchError {
                kind: usage.kind,
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

/// [`do_lock_local_editable`], with the spelling of the project's typed usage
/// of `respelled` not checked: the caller takes it from the project that the
/// usage resolved to, as recorded in the lock (see `sysand add`)
#[cfg(feature = "filesystem")]
pub fn do_lock_local_editable_respelling<
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
    ctx: &ProjectContext,
    respelled: &Identifier,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let project = EditableProject::new(
        path.as_ref().to_owned(),
        LocalSrcProject::new_access(
            wrapfs::canonicalize(&project_root).map_err(LockError::Io)?,
            None,
        ),
    );

    let base_path = ctx.project_root();
    lock_projects(
        [(identifiers, &project, base_path)],
        resolver,
        provided_usages,
        ctx,
        Some(respelled),
    )
}

/// Treats a project at `path` as an editable project and solves for its dependencies.
#[cfg(feature = "filesystem")]
/// Lock a whole workspace: every member, in one solve, into one lockfile.
///
/// `workspaces.md` "Locking": "Always a single lockfile ... for the whole
/// workspace in the workspace root. It records every project of the
/// solved graph by the identifier `pkg:sysand/<publisher>/<name>`,
/// members included."
///
/// `workspace` is loaded with
/// [`crate::workspace::model_toml::MemberChecks::All`]: the lockfile and
/// `.sysand` live at the workspace root, not in the members; see that
/// type's documentation.
///
/// **`ctx.sources_root()` must be the workspace root.** Every solved
/// local dependency's `src_path` is relativized against it, and one
/// lockfile whose paths meant "relative to whichever member ran `lock`"
/// would say different things in different lines. It is a `debug_assert`
/// rather than an error because it is a precondition on the caller.
#[cfg(feature = "filesystem")]
pub fn do_lock_workspace<PD: ProjectRead + Debug, R: ResolveRead<ProjectStorage = PD> + Debug>(
    workspace: &crate::workspace::model_toml::Workspace,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    lock_workspace(workspace, resolver, provided_usages, ctx, None)
}

/// [`do_lock_workspace`], with the spelling of the typed usage of
/// `respelled` that the members declare not checked (see
/// [`check_typed_usages`])
#[cfg(feature = "filesystem")]
pub fn do_lock_workspace_respelling<
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    workspace: &crate::workspace::model_toml::Workspace,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: &Identifier,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    lock_workspace(workspace, resolver, provided_usages, ctx, Some(respelled))
}

#[cfg(feature = "filesystem")]
fn lock_workspace<PD: ProjectRead + Debug, R: ResolveRead<ProjectStorage = PD> + Debug>(
    workspace: &crate::workspace::model_toml::Workspace,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<LockOutcome<PD>, LockProjectError<EditableLocalSrcProject, PD, R>> {
    debug_assert_eq!(
        ctx.sources_root().as_deref(),
        Some(workspace.root()),
        "a workspace lockfile's source paths are relative to its root"
    );

    // The base survives on `Member::project`, and is lost by the time
    // `do_lock_projects_with_bases` reads a project's usages through
    // `get_info` -- so the check belongs here, before anything is built.
    for member in workspace.members() {
        for located in &member.project.usages {
            if located.base == crate::manifest::usage::UsageBase::WorkspaceRoot
                && let Some(path) = located.usage.local_path()
            {
                return Err(LockProjectError::InheritedLocalUsage {
                    member: member.path.path.to_string(),
                    usage: located.usage.to_string(),
                    path: path.to_string(),
                });
            }
        }
    }

    let table = std::sync::Arc::new(workspace.table().clone());
    // The resolver reads a member's `dir` target as a project of its
    // own, and a member that inherits a key cannot be read without the
    // table. Applied here rather than by the caller: a caller that
    // forgot would see nothing wrong until some member inherited
    // something.
    let resolver = resolver.within_workspace(workspace.root().to_owned(), table.clone());
    let members: Vec<EditableLocalSrcProject> = workspace
        .members()
        .iter()
        .map(|member| {
            EditableProject::new(
                member.path.path.clone(),
                LocalSrcProject::new_member(member.dir.clone(), None, table.clone()),
            )
        })
        .collect();

    lock_projects(
        members
            .iter()
            .zip(workspace.members())
            .map(|(project, member)| {
                let purl = member
                    .project
                    .identity()
                    .lock_identifier()
                    .parse()
                    .expect("a validated publisher and name are a valid PURL");
                (Some(vec![purl]), project, Some(member.dir.clone()))
            }),
        resolver,
        provided_usages,
        ctx,
        respelled,
    )
}

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

    do_lock_projects([(identifiers, &project)], resolver, provided_usages, ctx)
}

#[cfg(test)]
#[path = "./lock_tests.rs"]
mod tests;
