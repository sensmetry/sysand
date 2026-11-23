// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
#[cfg(feature = "filesystem")]
use camino::Utf8Path;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::{self, Debug},
};
use thiserror::Error;

pub const DEFAULT_LOCKFILE_NAME: &str = "sysand-lock.toml";

use crate::{
    context::ProjectContext,
    graph::{MetamodelDirectionError, MetamodelReleaseMismatch, check_metamodel_pair},
    lock::{Lock, Project, Usage, hash_str},
    model::{
        InterchangeProjectUsage, InterchangeProjectValidationError, Metamodel, ProjectName,
        ProjectPublisher,
    },
    project::{
        ProjectDefinition, ProjectRead, SourceSymbolsError, SymbolIndexError,
        ValidatedDependencies, exported_symbols,
        utils::{FsIoError, Identifier},
    },
    resolve::ResolveRead,
    solve::pubgrub::{SolverError, solve_with_bases_cancellable},
    utils::ProvidedProjects,
};
#[cfg(feature = "filesystem")]
use crate::{
    project::{editable::EditableProject, local_src::LocalSrcProject, utils::wrapfs},
    workspace::model_toml::Workspace,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncompleteField {
    Info,
    CanonicalDigest,
}

impl fmt::Display for IncompleteField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("info"),
            Self::CanonicalDigest => f.write_str("canonical digest"),
        }
    }
}

#[derive(Error, Debug)]
pub enum LockProjectError<PI: ProjectRead, PD: ProjectRead, R: ResolveRead + Debug + 'static> {
    #[error(transparent)]
    InputProjectError(PI::Error),
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
    /// The usage is an index usage, and the entry the index serves for it
    /// declares a publisher and name that do not normalize to the usage's.
    pub invalid_index_entry: bool,
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
            invalid_index_entry,
        } = self;
        let usage = format!("{kind} usage `{usage_publisher}/{usage_name}` in {declared_by}");
        if *invalid_index_entry {
            let declared = publisher.as_deref().unwrap_or("<none>");
            return write!(
                f,
                "{usage} resolved to version {version} of `{declared}/{name}`, which is not \
                 the project the usage names;\n\
                 the index entry for `{usage_publisher}/{usage_name}` is invalid and has to be \
                 fixed by the index's maintainers"
            );
        }
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
                f.write_str("use a resource usage of it, naming its IRI, instead")
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
    #[error("failed to read the symbols of project {project_label}")]
    Symbols {
        project_label: String,
        #[source]
        source: Box<SourceSymbolsError>,
    },
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("incomplete project {project_label}: missing {field}")]
    IncompleteProject {
        /// Human-readable label for the project (an IRI when the project
        /// was resolved for a usage, a name+version when available locally, or
        /// `<unknown input project>` as a last resort).
        project_label: String,
        /// Which required piece was missing: `info`, `meta`, or
        /// `canonical digest`.
        field: IncompleteField,
    },
    /// The metamodel direction rule, over the graph the solve produced.
    #[error(transparent)]
    MetamodelDirection(Box<MetamodelDirectionError<String>>),
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
pub fn do_lock_projects<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    // TODO: maybe take Vec<Iri<String>>, since empty vec is tolerated
    I: IntoIterator<Item = (Option<Vec<Identifier>>, &'a PI)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<Lock, LockProjectError<PI, PD, R>> {
    #[cfg(feature = "filesystem")]
    let base_path = ctx.project_root();
    #[cfg(not(feature = "filesystem"))]
    let base_path = None;
    lock_projects(
        projects.into_iter().map(|(identifiers, project)| {
            (
                identifiers.map(|ids| ids.into_iter().map(Identifier::into_string).collect()),
                project,
                base_path.clone(),
                None,
            )
        }),
        resolver,
        provided_usages,
        ctx,
        None,
    )
}

/// Lock the input projects, each with its own base path: a workspace's
/// members are its input projects, and a member's `dir` usages are
/// relative to that member.
///
/// `ctx` supplies the root a solved dependency's sources are relativized
/// against. A `ctx` with neither a project nor a workspace panics.
///
/// The spelling of the typed usage of `respelled` that the projects
/// declare is not checked (see [`check_typed_usages`]). A project given
/// with its manifest already read is not read again.
fn lock_projects<
    'a,
    PI: ProjectRead + Debug + 'a,
    PD: ProjectRead + Debug,
    I: IntoIterator<
        Item = (
            Option<Vec<String>>,
            &'a PI,
            Option<camino::Utf8PathBuf>,
            Option<ProjectDefinition>,
        ),
    >,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    projects: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock, LockProjectError<PI, PD, R>> {
    let mut lock = Lock::default();
    let mut metamodels = HashMap::new();

    let mut all_deps = vec![];

    for (identifiers, project, base_path, definition) in projects {
        let input_project_label = || match identifiers.as_ref().and_then(|ids| ids.first()) {
            Some(iri) => format!("`{iri}`"),
            None => "<unknown input project>".to_owned(),
        };

        // Which project declared the usages below, for the conflicts a
        // failed solve reports. Anonymous when `identifiers` is `None`.
        // Taken before `identifiers` is moved below.
        let declarer = identifiers
            .as_ref()
            .and_then(|ids| ids.first())
            .map(ToString::to_string);

        let definition = match definition {
            Some(definition) => Some(definition),
            None => project
                .get_project()
                .map_err(LockProjectError::InputProjectError)?,
        }
        .ok_or_else(|| {
            LockProjectError::LockError(LockError::IncompleteProject {
                project_label: input_project_label(),
                field: IncompleteField::Info,
            })
        })?;
        let named_project_label = format!("`{}` {}", definition.name(), definition.version());
        let ValidatedDependencies {
            name: _,
            publisher: _,
            usages,
            metamodel,
        } = definition
            .validated_dependencies()
            .map_err(|e| LockError::InvalidProject {
                identifier: named_project_label.clone(),
                source: e,
            })?;
        let exports = exported_symbols(project, &definition).map_err(|e| match e {
            SymbolIndexError::Project(e) => LockProjectError::InputProjectError(e),
            SymbolIndexError::Source(source) => LockProjectError::LockError(LockError::Symbols {
                project_label: named_project_label.clone(),
                source: Box::new(source),
            }),
        })?;
        let sources = project
            .sources(ctx)
            .map_err(LockProjectError::InputProjectError)?;
        debug_assert_ne!(sources, []);

        metamodels.insert(lock.projects.len(), metamodel);
        lock.projects.push(Project {
            name: definition.name().to_owned(),
            publisher: definition.publisher().map(ToOwned::to_owned),
            version: definition.version(),
            exports,
            identifiers: identifiers.unwrap_or_default(),
            sources,
            usages: usages.iter().map(Usage::from).collect(),
        });

        all_deps.extend(usages.into_iter().map(|usage| {
            (
                crate::resolve::ResolutionInfo::new(usage, base_path.clone())
                    .declared_by(declarer.clone()),
                DeclaredBy::Input(named_project_label.clone()),
            )
        }));
    }

    Ok(lock_extend(
        lock,
        metamodels,
        all_deps,
        resolver,
        provided_usages,
        ctx,
        respelled,
    )?)
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
    lock: Lock,
    usages: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
) -> Result<Lock, LockError<PD, R>> {
    #[cfg(feature = "filesystem")]
    let base_path = ctx.project_root();
    #[cfg(not(feature = "filesystem"))]
    let base_path = None;
    lock_extend(
        lock,
        HashMap::new(),
        usages.into_iter().map(|(usage, declared_by)| {
            (
                crate::resolve::ResolutionInfo::new(usage, base_path.clone()),
                declared_by,
            )
        }),
        resolver,
        provided_usages,
        ctx,
        None,
    )
}

/// [`do_lock_extend`], with each usage's own base path, and with the
/// spelling of the typed usage of `respelled` among the inputs not checked
/// (see [`check_typed_usages`]).
///
/// A workspace is solved as one unit and its members do not share a
/// base: a usage's `dir` is relative to the member, a group entry's
/// relative to the workspace root.
///
/// `ctx` still supplies the root a solved dependency's sources are
/// relativized against, which is separate from the base its usages resolve
/// against. A `ctx` with neither a project nor a workspace panics.
///
/// `metamodels` maps an index into `lock.projects` to the metamodel that
/// project declares. The metamodel rules are checked over the edges whose
/// ends both have an entry, so an edge from a project of `lock` that has
/// none, and that the solve does not produce again, is not checked.
fn lock_extend<
    PD: ProjectRead + Debug,
    I: IntoIterator<Item = (crate::resolve::ResolutionInfo, DeclaredBy)>,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    mut lock: Lock,
    mut metamodels: HashMap<usize, Option<Metamodel>>,
    usages: I,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock, LockError<PD, R>> {
    let (inputs, declared_by): (Vec<_>, Vec<_>) = usages.into_iter().unzip();
    // Typed usages, to check against the projects they resolve to
    let mut typed_usages: Vec<(TypedSpelling, DeclaredBy)> = inputs
        .iter()
        .zip(declared_by)
        .filter_map(|(info, declared_by)| Some((TypedSpelling::of(info.usage())?, declared_by)))
        .collect();
    // Publisher, name and version of each solved project
    let mut solved = HashMap::new();
    let solution = solve_with_bases_cancellable(inputs, resolver, ctx.cancellation.clone())
        .map_err(LockError::Solver)?;
    let mut lock_projects = HashSet::new();
    let mut lock_symbols = HashMap::new();
    for (i, p) in lock.projects.iter().enumerate() {
        if let Some(id) = p.identifiers.first() {
            // FIXME: better deduplication. What to consider? Deduplicating by
            // canonical checksum would be rigorous, but that is not necessary,
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
        let definition = project
            .get_project()
            .map_err(LockError::DependencyProject)?
            .ok_or_else(|| LockError::IncompleteProject {
                project_label: identifier.to_string(),
                field: IncompleteField::Info,
            })?;
        let version = definition.version();
        let ValidatedDependencies {
            name: typed_name,
            publisher: typed_publisher,
            usages,
            metamodel,
        } = definition
            .validated_dependencies()
            .map_err(|e| LockError::InvalidProject {
                identifier: format!("`{identifier}` {version}"),
                source: e,
            })?;
        let exports = exported_symbols(&project, &definition).map_err(|e| match e {
            SymbolIndexError::Project(e) => LockError::DependencyProject(e),
            SymbolIndexError::Source(source) => LockError::Symbols {
                project_label: format!("`{identifier}` {version}"),
                source: Box::new(source),
            },
        })?;
        let name = definition.name().to_owned();
        let publisher = definition.publisher().map(ToOwned::to_owned);

        for usage in &usages {
            if let Some(spelling) = TypedSpelling::of(usage) {
                typed_usages.push((
                    spelling,
                    DeclaredBy::Dependency(format!("`{name}` {version} (`{identifier}`)")),
                ));
            }
        }
        solved.insert(
            identifier.clone(),
            (typed_publisher, typed_name, version.clone()),
        );

        let sources = if provided_usages.contains_key(&identifier) {
            Vec::new()
        } else {
            let sources = project.sources(ctx).map_err(LockError::DependencyProject)?;
            debug_assert_ne!(sources, []);
            sources
        };

        let lock_project = Project {
            name,
            publisher,
            version,
            exports,
            identifiers: vec![identifier.to_string()],
            sources,
            // TODO: into_iter
            usages: usages.iter().map(Usage::from).collect(),
        };
        if lock_projects.contains(identifier.as_str()) {
            log::debug!(
                "not adding project `{identifier}` ({}) to lock, as lock already contains it",
                lock_project.version
            );
            if let Some(idx) = lock
                .projects
                .iter()
                .position(|p| p.identifiers.first() == lock_project.identifiers.first())
            {
                metamodels.entry(idx).or_insert(metamodel);
            }
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
            metamodels.insert(new_idx, metamodel);
            lock.projects.push(lock_project);
        }
    }

    check_typed_usages(typed_usages, &solved, respelled).map_err(LockError::TypedUsageMismatch)?;

    for mismatch in
        check_metamodel_edges(&lock, &metamodels).map_err(LockError::MetamodelDirection)?
    {
        log::warn!("{mismatch}");
    }

    Ok(lock)
}

/// The metamodel rules (direction and release) of every edge of `lock`
/// whose ends both have an entry in `metamodels`, which maps an index into
/// `lock.projects` to the metamodel that project declares. An edge is a
/// usage naming the first identifier of the project satisfying it.
///
/// The release mismatches, which are warnings, are returned for the caller
/// to report; the direction error is the error.
fn check_metamodel_edges(
    lock: &Lock,
    metamodels: &HashMap<usize, Option<Metamodel>>,
) -> Result<Vec<MetamodelReleaseMismatch<String>>, Box<MetamodelDirectionError<String>>> {
    let by_first_identifier: HashMap<&str, usize> = lock
        .projects
        .iter()
        .enumerate()
        .filter_map(|(idx, project)| Some((project.identifiers.first()?.as_str(), idx)))
        .collect();

    let mut mismatches = Vec::new();
    for (idx, project) in lock.projects.iter().enumerate() {
        let Some(metamodel) = metamodels.get(&idx) else {
            continue;
        };
        for usage in &project.usages {
            let Some(&dep_idx) = by_first_identifier.get(usage.inner()) else {
                continue;
            };
            let Some(dep_metamodel) = metamodels.get(&dep_idx) else {
                continue;
            };
            if let Some(mismatch) = check_metamodel_pair(
                (&project.label(), metamodel.as_ref()),
                (&lock.projects[dep_idx].label(), dep_metamodel.as_ref()),
            )? {
                mismatches.push(mismatch);
            }
        }
    }
    Ok(mismatches)
}

/// A typed usage's kind, publisher and name
struct TypedSpelling {
    /// With an article, see
    /// [`crate::model::InterchangeProjectUsageG::kind_with_article`]
    kind: &'static str,
    publisher: String,
    name: String,
    /// What the usage resolves under: the publisher and name for most typed
    /// usages, the IRI for a remote KPAR usage
    identifier: Identifier,
    /// Whether the usage is an index usage
    index: bool,
}

impl TypedSpelling {
    /// `None` for a resource usage and a standard-library remote KPAR usage,
    /// which name no publisher
    fn of(usage: &InterchangeProjectUsage) -> Option<Self> {
        let (publisher, name) = usage.typed_publisher_name()?;
        Some(Self {
            kind: usage.kind_with_article(),
            publisher: publisher.to_owned(),
            name: name.to_owned(),
            identifier: Identifier::from(usage),
            index: matches!(
                usage,
                InterchangeProjectUsage::Index {
                    publisher: _,
                    name: _,
                    version_constraint: _,
                }
            ),
        })
    }
}

/// Check that each typed usage's publisher and name are spelled exactly as
/// the project it resolved to (in `solved`, as publisher, name and version)
/// spells them. Normalization makes them resolve to the same project
/// regardless, so this is the only place a misspelling is caught.
///
/// The typed usage of `respelled` that the projects being locked declare
/// takes its spelling from the project it resolved to, so only that
/// project's identity is checked: it declares a publisher, and its
/// publisher and name normalize to the usage's. Dependencies' usages of it
/// are checked as any other.
fn check_typed_usages(
    typed_usages: Vec<(TypedSpelling, DeclaredBy)>,
    solved: &HashMap<Identifier, (Option<ProjectPublisher>, ProjectName, String)>,
    respelled: Option<&Identifier>,
) -> Result<(), Box<TypedUsageMismatchError>> {
    for (usage, declared_by) in typed_usages {
        // Not being in the solution is not a mismatch: e.g. a project the
        // caller provides, or one already in the lock
        let Some((publisher, name, version)) = solved.get(&usage.identifier) else {
            continue;
        };
        let same_identity = publisher
            .as_ref()
            .is_some_and(|p| Identifier::from_project(p, name) == usage.identifier);
        let respelling =
            matches!(declared_by, DeclaredBy::Input(_)) && respelled == Some(&usage.identifier);
        let accepted = if respelling {
            same_identity
        } else {
            publisher.as_ref().map(ProjectPublisher::as_str) == Some(usage.publisher.as_str())
                && name.as_str() == usage.name
        };
        if !accepted {
            // An index entry is found by the usage's identifier, so one that
            // declares a different identity is the index's error.
            let invalid_index_entry = usage.index && !same_identity;
            return Err(Box::new(TypedUsageMismatchError {
                invalid_index_entry,
                kind: usage.kind,
                usage_publisher: usage.publisher,
                usage_name: usage.name,
                declared_by,
                version: version.clone(),
                publisher: publisher.as_ref().map(|p| p.as_str().to_owned()),
                name: name.as_str().to_owned(),
            }));
        }
    }
    Ok(())
}

#[cfg(feature = "filesystem")]
pub type EditableLocalSrcProject = EditableProject<LocalSrcProject>;

/// Lock a whole workspace: every member, in one solve, into one lockfile.
///
/// The lockfile is written in the workspace root. It records every project of
/// the solved graph by the identifier `pkg:sysand/<publisher>/<name>`, members
/// included.
///
/// **`ctx.sources_root()` must be the workspace root.** Every solved
/// local dependency's `src_path` is relativized against it. Checked with
/// a `debug_assert`.
///
/// The spelling of the typed usage of `respelled` that the members declare
/// is not checked (see [`check_typed_usages`]).
#[cfg(feature = "filesystem")]
pub fn do_lock_workspace<PD: ProjectRead + Debug, R: ResolveRead<ProjectStorage = PD> + Debug>(
    workspace: &Workspace,
    resolver: R,
    provided_usages: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock, LockProjectError<EditableLocalSrcProject, PD, R>> {
    debug_assert_eq!(
        ctx.sources_root().as_deref(),
        Some(workspace.root()),
        "a workspace lockfile's source paths are relative to its root"
    );

    // A member that inherits keys cannot be read without its workspace.
    let resolver = resolver.within_workspace(std::sync::Arc::new(workspace.clone()));
    let members: Vec<EditableLocalSrcProject> = workspace
        .members()
        .iter()
        .map(|member| {
            EditableProject::new(
                member.path.path.clone(),
                LocalSrcProject::new_member(member.dir.clone(), None, member.project.clone()),
            )
        })
        .collect();

    lock_projects(
        members
            .iter()
            .zip(workspace.members())
            .map(|(project, member)| {
                let purl = member.project.identity().lock_identifier();
                let definition = ProjectDefinition::Toml(Box::new(member.project.clone()));
                (
                    Some(vec![purl]),
                    project,
                    Some(member.dir.clone()),
                    Some(definition),
                )
            }),
        resolver,
        provided_usages,
        ctx,
        respelled,
    )
}

/// Treats the project at `project_root` as an editable project, at `.`, and
/// solves for its dependencies.
///
/// The spelling of the project's typed usage of `respelled` is not checked:
/// it is taken from the project that the usage resolved to, as recorded in
/// the lock.
#[cfg(feature = "filesystem")]
pub fn do_lock_local_editable<
    PR: AsRef<Utf8Path>,
    PD: ProjectRead + Debug,
    R: ResolveRead<ProjectStorage = PD> + Debug,
>(
    project_root: PR,
    identifiers: Option<Vec<Identifier>>,
    provided_usages: &ProvidedProjects,
    resolver: R,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock, LockProjectError<EditableLocalSrcProject, PD, R>> {
    let project = EditableProject::new(
        ".".into(),
        LocalSrcProject::new_access(
            wrapfs::canonicalize(&project_root).map_err(LockError::Io)?,
            None,
        ),
    );

    lock_projects(
        [(
            identifiers.map(|ids| ids.into_iter().map(Identifier::into_string).collect()),
            &project,
            ctx.project_root(),
            ctx.current_definition().cloned(),
        )],
        resolver,
        provided_usages,
        ctx,
        respelled,
    )
}

#[cfg(test)]
#[path = "./lock_tests.rs"]
mod tests;
