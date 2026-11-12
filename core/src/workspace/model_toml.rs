// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A workspace read from the `[workspace]` table of a `model.toml`.
//!
//! A load reads the table, checks the members (their requirements and
//! stray per-member state) and resolves each member's manifest against
//! the table it belongs to. Standalone resolution
//! ([`crate::manifest::kpar_view::resolved_of_text`]) refuses a member
//! carrying `version.workspace = true`.

use crate::paths::{RelativizePathError, is_above_root, join_lexical, relativize_path};
use std::collections::{HashMap, hash_map::Entry};
use std::sync::Arc;

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::commands::build::DEFAULT_OUTPUT_NAME;
use crate::commands::lock::DEFAULT_LOCKFILE_NAME;
use crate::discover::{Ancestry, DirManifest, Seek};
use crate::env::DEFAULT_ENV_NAME;
use crate::graph::DependencyGraph;
use crate::manifest::diagnostics::{Diagnostic, Diagnostics, Validated};
use crate::manifest::project::ResolvedProject;
use crate::manifest::usage::Usage;
use crate::manifest::workspace_table::{MemberPath, Region, WorkspaceTable};
use crate::manifest::{MODEL_TOML_FILE, ModelToml, ModelTomlKind};
use crate::project::ProjectFormat;
use crate::project::local_src::{detect_format, json_manifest};
use crate::project::utils::{FsIoError, ProjectDeserializationError, validate_manifest, wrapfs};

/// Why a load failed, split by the file each half is about.
///
/// A [`crate::manifest::diagnostics::Report`] is about one file, so the
/// diagnostics of the root manifest and of each member's manifest are kept
/// apart, each under the name of the file it points into.
///
/// Both halves are filled before the load gives up.
#[derive(Debug)]
pub struct LoadFailure {
    /// Member requirements and stray per-member state, each positioned
    /// at the `members` entry that named the directory. `None` when only
    /// members' own files are bad.
    pub root: Option<ProjectDeserializationError>,
    /// Members whose own `model.toml` did not read or did not resolve,
    /// each naming its own file.
    pub members: Vec<ProjectDeserializationError>,
}

impl std::fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for error in self.root.iter().chain(self.members.iter()) {
            if !first {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
            first = false;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// The root's own `model.toml` is not a readable manifest.
    #[error(transparent)]
    Root(ProjectDeserializationError),
    /// It reads, but it defines a project: the `model.toml` that defines
    /// the workspace must not also contain a `[project]` table.
    #[error("`{0}` defines a project, not a workspace")]
    NotAWorkspace(Utf8PathBuf),
    #[error("{0}")]
    Invalid(Box<LoadFailure>),
}

/// One member of a workspace.
#[derive(Debug, Clone)]
pub struct Member {
    /// The `members` entry that named it: both the spelling the author
    /// wrote and the path normalized against the root.
    pub path: MemberPath,
    /// The member's directory, absolute.
    pub dir: Utf8PathBuf,
    /// Its manifest, resolved against the workspace table.
    pub project: ResolvedProject,
}

#[derive(Debug)]
pub struct Workspace {
    root: Utf8PathBuf,
    table: Arc<WorkspaceTable>,
    members: Vec<Member>,
}

/// What a load checks beyond reading the members.
///
/// A member has four requirements. The fourth, no lockfile, `.sysand` or
/// `output` of its own, is skipped with [`MemberChecks::SkipStrayState`].
///
/// The check runs only when a workspace is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberChecks {
    /// Every member requirement.
    All,
    /// All but the requirement of no per-member lockfile, `.sysand` or
    /// `output`.
    SkipStrayState,
}

impl Workspace {
    /// Read the `model.toml` at `root` as a workspace, check what `checks`
    /// asks for and resolve each member's manifest against the table.
    ///
    /// `root` is canonicalized. Files are named relative to the root in
    /// every report: `model.toml` for the root, `app/model.toml` for a
    /// member.
    pub fn load_with(root: impl AsRef<Utf8Path>, checks: MemberChecks) -> Result<Self, LoadError> {
        let root = wrapfs::canonicalize(root.as_ref())?;
        let text = wrapfs::read_to_string(root.join(MODEL_TOML_FILE))?;
        let manifest = ModelToml::parse(MODEL_TOML_FILE, &text);
        let table = match validate_manifest(MODEL_TOML_FILE, manifest).map_err(LoadError::Root)? {
            ModelToml::Workspace(table) => *table,
            ModelToml::Project(_) => return Err(LoadError::NotAWorkspace(root)),
        };
        Self::with_table(root, Arc::new(table), checks)
    }

    /// [`Workspace::load_with`] for a root whose `[workspace]` table was
    /// already read. `root` must be canonical.
    pub fn with_table(
        root: Utf8PathBuf,
        table: Arc<WorkspaceTable>,
        checks: MemberChecks,
    ) -> Result<Self, LoadError> {
        let mut report = Diagnostics::new();
        let mut broken = Vec::new();
        let mut members = Vec::with_capacity(table.members.len());
        for path in &table.members {
            if let Some(member) =
                load_member(&root, path, &table, checks, &mut report, &mut broken)?
            {
                members.push(member);
            }
        }

        // Only over the members that loaded: a cycle running through a
        // broken member is invisible here, and its own diagnostic comes
        // first.
        report.extend(cross_member_checks(&members));

        if report.is_empty() && broken.is_empty() {
            return Ok(Self {
                root,
                table,
                members,
            });
        }
        report.sort_by_position();
        Err(LoadError::Invalid(Box::new(LoadFailure {
            root: (!report.is_empty())
                .then(|| ProjectDeserializationError::invalid(MODEL_TOML_FILE, report)),
            members: broken,
        })))
    }

    /// Replace the manifest of the member at `index` with `project`, its
    /// manifest as just read, and run the checks that span members over
    /// the members in hand.
    ///
    /// The member is replaced even when a check fails, so the workspace
    /// keeps matching the files.
    pub fn replace_member(
        &mut self,
        index: usize,
        project: ResolvedProject,
    ) -> Result<(), LoadError> {
        self.members[index].project = project;
        let mut report = Diagnostics::new();
        report.extend(cross_member_checks(&self.members));
        if report.is_empty() {
            return Ok(());
        }
        report.sort_by_position();
        Err(LoadError::Invalid(Box::new(LoadFailure {
            root: Some(ProjectDeserializationError::invalid(
                MODEL_TOML_FILE,
                report,
            )),
            members: Vec::new(),
        })))
    }

    /// The index of the member whose directory is `dir`, if there is one.
    pub fn member_index_of(
        &self,
        dir: impl AsRef<Utf8Path>,
    ) -> Result<Option<usize>, Box<FsIoError>> {
        let dir = wrapfs::canonicalize(dir.as_ref())?;
        Ok(self.members.iter().position(|member| member.dir == dir))
    }

    /// The member `--member <path-or-name>` names.
    ///
    /// The selector matches a `members` path or a member's `name`, both
    /// literally, so a value that matches one member's path and another's
    /// name is an ambiguity error naming both. A member named both ways
    /// counts once.
    ///
    /// A path matches against the spelling the author wrote.
    pub fn member(&self, selector: &str) -> Result<&Member, SelectError> {
        let mut matched: Vec<(&Member, &'static str)> = Vec::new();
        for member in &self.members {
            if member.path.raw == selector {
                matched.push((member, "the path"));
            } else if member.project.name.as_str() == selector {
                matched.push((member, "the name of the member at"));
            }
        }
        match matched.as_slice() {
            [(member, _)] => Ok(member),
            [] => Err(SelectError::NoSuchMember {
                root: self.root.clone(),
                selector: selector.to_owned(),
                known: if self.members.is_empty() {
                    "no members".to_owned()
                } else {
                    self.members
                        .iter()
                        .map(|member| format!("`{}`", member.path.raw))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            }),
            _ => Err(SelectError::AmbiguousMember {
                selector: selector.to_owned(),
                matches: matched
                    .iter()
                    .map(|(member, how)| format!("{how} `{}`", member.path.raw))
                    .collect::<Vec<_>>()
                    .join(" and "),
            }),
        }
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn table(&self) -> &Arc<WorkspaceTable> {
        &self.table
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// The members with each one after the members its `dir` usages name.
    ///
    /// A load refuses every cycle that is not allowed, so only members
    /// that all carry `stdlib` can be out of order, in `members` order.
    pub fn members_in_dependency_order(&self) -> Vec<&Member> {
        let graph = member_graph(&self.members);
        let nodes = graph.nodes();
        graph
            .dependency_order()
            .into_iter()
            .filter_map(|node| nodes.iter().position(|n| n == node))
            .map(|index| &self.members[index])
            .collect()
    }
}

/// Why discovery failed.
///
/// A nesting error and an unlisted project have no span to point at. The
/// variants about a file carry a report over that file.
#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// A `model.toml` on the walk that could not be read.
    #[error(transparent)]
    Manifest(ProjectDeserializationError),
    /// A workspace inside another that does not `exclude` it.
    #[error(
        "the workspace at `{inner}` lies inside the workspace at `{outer}`, which does not \
         `exclude` it; nested workspaces are not supported"
    )]
    Nested {
        inner: Utf8PathBuf,
        outer: Utf8PathBuf,
    },
    /// A project under the root that is in neither list.
    #[error(
        "the project at `{project}` lies under the workspace at `{root}` but is neither listed \
         in `workspace.members` nor under `workspace.exclude`; add it to one of the two"
    )]
    Unlisted {
        project: Utf8PathBuf,
        root: Utf8PathBuf,
    },
    /// [`start_region`] was given a directory the workspace does not
    /// contain.
    #[error("`{dir}` is not under the workspace at `{root}`")]
    NotUnderRoot { dir: Utf8PathBuf, root: Utf8PathBuf },
    #[error(transparent)]
    Relativize(#[from] RelativizePathError),
    /// The target is the whole workspace, and the command needs a
    /// project.
    #[error(
        "this command needs a project; run it from a member directory or pass \
         `--member <path-or-name>`"
    )]
    NeedsProject,
}

/// Why `--member <path-or-name>` did not pick one member.
#[derive(Debug, Error)]
pub enum SelectError {
    #[error("the workspace at `{root}` has no member `{selector}`; it has {known}")]
    NoSuchMember {
        root: Utf8PathBuf,
        selector: String,
        known: String,
    },
    #[error("`{selector}` is ambiguous: it matches {matches}")]
    AmbiguousMember { selector: String, matches: String },
}

/// Which part of a workspace a command was asked to act on.
///
/// `--workspace` forces all members, `--member <path-or-name>` picks one,
/// and with neither the starting directory decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector<'a> {
    /// What the starting directory implies.
    Current,
    /// Every member.
    Workspace,
    /// One member, named by its `members` entry as written or by its
    /// project name; see [`Workspace::member`].
    Member(&'a str),
}

/// Where a command run in `from` starts, answered from the table alone.
///
/// A path comparison against [`WorkspaceTable::region`] that does not load
/// the members, so it works where a load would fail, such as in a member
/// holding a lockfile, `.sysand/` or `output/` its own previous run
/// created.
///
/// Run from the workspace root or from a non-member directory under it
/// that defines no project, the whole workspace is the command's target;
/// run from a member (sub)dir, only that member is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartRegion {
    /// Inside `table.members[index]`.
    Member(usize),
    /// A project under an `exclude`d directory: the only kind of
    /// non-member project that may sit under a workspace root. The command
    /// acts on the project alone, a standalone project with its own
    /// lockfile, `.sysand` and `output`.
    Standalone(Utf8PathBuf),
    /// The root itself, or a directory under it that defines no project.
    /// The workspace root is never a member.
    Workspace,
}

/// Classifies a project under a workspace root, over a
/// [`WorkspaceTable`] and a root path.
///
/// The search for the project `from` sits in is bounded at the root: a
/// project above the root is a different project.
///
/// `root` must be the nearest workspace root above `from`, as discovery
/// finds it: no other `[workspace]` lies between them, so any manifest on
/// the way is a project's, and its presence is all that is checked.
///
/// `root` must be canonical ([`discover_table`] and [`Workspace::load_with`]
/// both canonicalize what they return), since `from` is canonicalized here
/// and the two are compared.
pub fn start_region(
    root: &Utf8Path,
    table: &WorkspaceTable,
    from: impl AsRef<Utf8Path>,
) -> Result<StartRegion, DiscoverError> {
    let from = wrapfs::canonicalize(from.as_ref())?;
    let mut current: &Utf8Path = &from;
    loop {
        if current == root {
            return Ok(StartRegion::Workspace);
        }
        if detect_format(current)? != ProjectFormat::Neither {
            return match region_of(root, table, current) {
                PathRegion::Member(index) => Ok(StartRegion::Member(index)),
                PathRegion::Excluded => Ok(StartRegion::Standalone(current.to_owned())),
                PathRegion::Unlisted => Err(DiscoverError::Unlisted {
                    project: current.to_owned(),
                    root: root.to_owned(),
                }),
                PathRegion::Outside => Err(DiscoverError::NotUnderRoot {
                    dir: current.to_owned(),
                    root: root.to_owned(),
                }),
            };
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => {
                return Err(DiscoverError::NotUnderRoot {
                    dir: from,
                    root: root.to_owned(),
                });
            }
        }
    }
}

/// The workspace `from` belongs to, as its `[workspace]` table.
///
/// The members are not loaded, so creating a new project is not blocked
/// by an unrelated member's mistake.
pub fn discover_table(
    from: impl AsRef<Utf8Path>,
) -> Result<Option<(Utf8PathBuf, WorkspaceTable)>, DiscoverError> {
    table_of(Ancestry::walk(from, Seek::Workspace)?)
}

/// [`discover_table`] over a walk already made.
///
/// The steps are pass 1 (the nearest `[workspace]`), the parse of its
/// table, and [`confirm_root`], in that order. A file carrying both tables
/// counts as a workspace on the walk, so a candidate that is one fails as
/// that ([`DiscoverError::Manifest`]), not as nested workspaces.
pub fn table_of(
    ancestry: Ancestry,
) -> Result<Option<(Utf8PathBuf, WorkspaceTable)>, DiscoverError> {
    let Some(candidate) = Candidate::find(ancestry)? else {
        return Ok(None);
    };
    let root = candidate.root;
    let table = workspace_table(&root.file, root.manifest)?;
    confirm_root(&root.dir, candidate.above)?;
    Ok(Some((root.dir, table)))
}

/// The nearest `[workspace]` a walk found: the candidate workspace root.
///
/// A candidate, because it may be nested (see [`confirm_root`]) and, as a
/// file carrying both tables counts as a workspace, may not be readable.
struct Candidate {
    root: WorkspaceFile,
    /// The rest of the walk, above the root.
    above: Above,
}

/// A directory whose `model.toml` names a `[workspace]`, and its parse.
struct WorkspaceFile {
    dir: Utf8PathBuf,
    file: Utf8PathBuf,
    manifest: Result<Validated<ModelToml>, Diagnostics>,
}

/// What a walk found above a directory, and the read that ended it early.
struct Above {
    found: std::vec::IntoIter<(Utf8PathBuf, DirManifest)>,
    failed: Option<Box<FsIoError>>,
}

impl Candidate {
    /// Pass 1: the first directory of the walk whose `model.toml` names
    /// a `[workspace]`.
    ///
    /// A file that is not TOML ends the walk with its error, as treating
    /// it as "not a workspace" would read every member under it as a
    /// standalone project. A file with neither table is walked past.
    fn find(ancestry: Ancestry) -> Result<Option<Self>, DiscoverError> {
        let (found, failed) = ancestry.into_found();
        let mut above = Above {
            found: found.into_iter(),
            failed,
        };
        Ok(above.next_workspace()?.map(|root| Self { root, above }))
    }
}

impl Above {
    /// The next directory whose `model.toml` names a `[workspace]`.
    fn next_workspace(&mut self) -> Result<Option<WorkspaceFile>, DiscoverError> {
        for (dir, manifest) in self.found.by_ref() {
            let DirManifest::ModelToml { file, parsed } = manifest else {
                continue;
            };
            match parsed.kind {
                ModelTomlKind::Workspace => {
                    return Ok(Some(WorkspaceFile {
                        dir,
                        file,
                        manifest: parsed.manifest,
                    }));
                }
                ModelTomlKind::NotToml => {
                    return Err(DiscoverError::Manifest(
                        ProjectDeserializationError::invalid(
                            &file,
                            parsed.manifest.err().unwrap_or_default(),
                        ),
                    ));
                }
                ModelTomlKind::Project | ModelTomlKind::Neither => {}
            }
        }
        match self.failed.take() {
            Some(error) => Err(DiscoverError::Io(error)),
            None => Ok(None),
        }
    }
}

/// The `[workspace]` table of a parsed `model.toml`.
///
/// Parsed, not loaded: the members are not checked, so a command inside a
/// separate workspace does not fail because a neighbour's member is
/// missing. The table itself must validate.
fn workspace_table(
    file: &Utf8Path,
    manifest: Result<Validated<ModelToml>, Diagnostics>,
) -> Result<WorkspaceTable, DiscoverError> {
    let manifest = validate_manifest(file, manifest).map_err(DiscoverError::Manifest)?;
    manifest.into_workspace(file.as_str()).map_err(|problem| {
        DiscoverError::Manifest(ProjectDeserializationError::invalid(file, problem))
    })
}

/// Pass 2: refuse a candidate nested in another workspace.
///
/// A second `[workspace]` table found above the first means the candidate
/// is inside another workspace. That is allowed only if the outer
/// workspace lists the candidate's directory, or a directory above it, in
/// `exclude`. Only the first outer is consulted.
fn confirm_root(candidate: &Utf8Path, mut above: Above) -> Result<(), DiscoverError> {
    let Some(outer) = above.next_workspace()? else {
        return Ok(());
    };
    let table = workspace_table(&outer.file, outer.manifest)?;
    let relative = relativize_path(candidate, &outer.dir)?;
    if table.excluding(&relative).is_some() {
        Ok(())
    } else {
        Err(DiscoverError::Nested {
            inner: candidate.to_owned(),
            outer: outer.dir,
        })
    }
}

/// Whether `path` is there, and if so whether it is a directory.
///
/// Distinguishes an absent entry from a file, which `wrapfs::is_dir`
/// cannot: they are different mistakes.
fn entry_kind(path: &Utf8Path) -> Result<Option<bool>, Box<FsIoError>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.is_dir())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Box::new(FsIoError::IsFile(path.into(), err))),
    }
}

/// The member requirements, and the read that answers them.
///
/// [`detect_format`] tests file existence only. A `model.toml` holding a
/// `[workspace]` table, one that is not TOML, and one defining neither
/// table all come back [`ProjectFormat::Toml`], so the parse is the check.
fn load_member(
    root: &Utf8Path,
    path: &MemberPath,
    table: &WorkspaceTable,
    checks: MemberChecks,
    report: &mut Diagnostics,
    broken: &mut Vec<ProjectDeserializationError>,
) -> Result<Option<Member>, Box<FsIoError>> {
    let raw = &path.raw;
    let dir = join_lexical(root, Utf8Path::new(path.path.as_str()));

    let mut refuse = |message: String| {
        report.push(Diagnostic::error(message).at_opt(path.at.clone()));
    };

    match entry_kind(&dir)? {
        None => {
            refuse(format!(
                "member `{raw}` does not exist; there is no directory at `{dir}`"
            ));
            return Ok(None);
        }
        Some(false) => {
            refuse(format!("member `{raw}` is not a directory"));
            return Ok(None);
        }
        Some(true) => {}
    }

    // Checked before the format so that a member with both a stray lockfile
    // and the wrong manifest format reports both.
    if checks == MemberChecks::All {
        stray_state(root, path, &dir, report)?;
    }

    let has_toml = wrapfs::is_file(dir.join(MODEL_TOML_FILE))?;
    match (has_toml, json_manifest(&dir)?) {
        (false, Some(json)) => {
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` uses `{json}`; a workspace member must use the `model.toml` \
                     project definition (`sysand migrate {raw}`)"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        (true, Some(json)) => {
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` has both `model.toml` and `{json}`; it is unclear whether it \
                     is a non-zipped KPAR or a development project; remove the one that does not \
                     define the project"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        (false, None) => {
            report.push(
                Diagnostic::error(format!("member `{raw}` defines no project"))
                    .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        (true, None) => {
            let file = format!("{}/{MODEL_TOML_FILE}", path.path);
            let text = wrapfs::read_to_string(dir.join(MODEL_TOML_FILE))?;
            let manifest = match validate_manifest(&file, ModelToml::parse(&file, &text)) {
                Ok(manifest) => manifest,
                Err(error) => {
                    broken.push(error);
                    return Ok(None);
                }
            };
            let project = match manifest {
                ModelToml::Project(project) => *project,
                ModelToml::Workspace(_) => {
                    report.push(
                        Diagnostic::error(format!(
                            "member `{raw}` defines a workspace, not a project; a member cannot \
                             be a workspace root"
                        ))
                        .at_opt(path.at.clone()),
                    );
                    return Ok(None);
                }
            };
            match validate_manifest(&file, project.resolve(Some(table))) {
                Ok(project) => Ok(Some(Member {
                    path: path.clone(),
                    dir,
                    project,
                })),
                Err(error) => {
                    broken.push(error);
                    Ok(None)
                }
            }
        }
    }
}

/// The checks that span members, over the members that loaded.
fn cross_member_checks(members: &[Member]) -> Vec<Diagnostic> {
    let mut found = duplicate_identities(members);
    found.extend(member_cycles(members));
    found
}

/// Two members that are the same project.
///
/// The lockfile holds one entry per identity, so two members claiming one
/// identity cannot be locked. Identities are compared normalized.
fn duplicate_identities(members: &[Member]) -> Vec<Diagnostic> {
    let mut seen: HashMap<_, &Member> = HashMap::new();
    let mut found = Vec::new();
    for member in members {
        match seen.entry(member.project.identity()) {
            Entry::Vacant(slot) => {
                slot.insert(member);
            }
            Entry::Occupied(first) => {
                let identity = member.project.identity();
                found.push(
                    Diagnostic::error(format!(
                        "members `{}` and `{}` name the same project `{}/{}`; two members \
                         whose publisher and name normalize to the same value are a duplicate",
                        first.get().path.raw,
                        member.path.raw,
                        identity.publisher,
                        identity.name
                    ))
                    // At the second entry.
                    .at_opt(member.path.at.clone()),
                );
            }
        }
    }
    found
}

/// The cycles among the members' `dir` usages.
///
/// Only `dir` usages are edges, joined onto the member's own directory
/// and canonicalized, so that a member's `dir = "../<itself>"` is a
/// self-cycle. A `kpar-path` names a `.kpar` file rather than a member
/// directory. `located.base` is not consulted: an inherited path usage is
/// not an edge between members.
fn member_cycles(members: &[Member]) -> Vec<Diagnostic> {
    let graph = member_graph(members);
    let by_dir: HashMap<&Utf8PathBuf, &Member> = graph.nodes().iter().zip(members).collect();
    let stdlib = |dir: &Utf8PathBuf| {
        by_dir
            .get(dir)
            .is_some_and(|member| member.project.stdlib.is_some())
    };

    let mut diagnostics = Vec::new();
    for cycle in graph.cycles() {
        // A cycle is allowed if every project on it carries `stdlib`.
        if cycle.component.iter().all(stdlib) {
            continue;
        }
        let named = cycle
            .path
            .iter()
            .map(|dir| {
                by_dir
                    .get(dir)
                    .map_or_else(|| dir.to_string(), |member| member.path.raw.clone())
            })
            .collect::<Vec<_>>()
            .join("` -> `");
        let at = cycle
            .path
            .first()
            .and_then(|dir| by_dir.get(dir))
            .and_then(|member| member.path.at.clone());
        diagnostics.push(
            Diagnostic::error(format!(
                "dependency cycle: `{named}`; only projects that carry `stdlib` may form a cycle"
            ))
            .at_opt(at),
        );
    }
    diagnostics
}

/// The members' `dir` usages of each other, as a graph over their
/// directories, canonical where the filesystem allows it. Node `i` is
/// `members[i]`.
///
/// A usage of a project outside the workspace is no edge.
fn member_graph(members: &[Member]) -> DependencyGraph<Utf8PathBuf> {
    let canonical = |path: Utf8PathBuf| wrapfs::canonicalize(&path).unwrap_or(path);
    let mut graph = DependencyGraph::new();
    for member in members {
        graph.add_node(canonical(member.dir.clone()));
    }
    let nodes = graph.nodes().to_vec();
    for (member, node) in members.iter().zip(&nodes) {
        for located in &member.project.usages {
            if let Usage::SourcePath {
                dir,
                publisher: _,
                name: _,
            } = &located.usage
            {
                graph.add_edge(node, &canonical(node.join(dir.as_str())));
            }
        }
    }
    graph
}

/// Three existence tests; no content is read.
///
/// The message names the root, where the state belongs. Excluded
/// directories are not checked: standalone projects under them keep their
/// state.
fn stray_state(
    root: &Utf8Path,
    path: &MemberPath,
    dir: &Utf8Path,
    report: &mut Diagnostics,
) -> Result<(), Box<FsIoError>> {
    let state = [
        (
            DEFAULT_LOCKFILE_NAME,
            "lockfile",
            wrapfs::is_file(dir.join(DEFAULT_LOCKFILE_NAME))?,
        ),
        (
            DEFAULT_ENV_NAME,
            "environment",
            wrapfs::is_dir(dir.join(DEFAULT_ENV_NAME))?,
        ),
        (
            DEFAULT_OUTPUT_NAME,
            "build output directory",
            wrapfs::is_dir(dir.join(DEFAULT_OUTPUT_NAME))?,
        ),
    ];
    for (name, what, present) in state {
        if present {
            report.push(
                Diagnostic::error(format!(
                    "member `{}` has its own `{name}`; a workspace has one {what} at `{root}`; \
                     remove the member's copy",
                    path.raw
                ))
                .at_opt(path.at.clone()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "./model_toml_tests.rs"]
mod tests;

/// A [`crate::project::local_src::LocalSrcProject`] for a path a caller
/// gave, rather than a directory that was walked to.
///
/// A member's manifest resolves only against its workspace's table, so
/// reading one without it fails with "`metamodel` inherits from group
/// `<g>`, but the project is not a member of any workspace". This finds
/// the table for a path the way discovery does for a working directory.
///
/// Built from [`discover_table`] and [`region_of`], which matches a
/// member's directory exactly, so a project merely inside one is not
/// handed its table.
///
/// An excluded project, and one outside any workspace, read table-less as
/// standalone. An unlisted one is refused, as when walking up from inside
/// it; see [`PathRegion`].
pub fn project_at_path(
    path: impl Into<Utf8PathBuf>,
    nominal_path: Option<typed_path::Utf8UnixPathBuf>,
) -> Result<crate::project::local_src::LocalSrcProject, DiscoverError> {
    let path = path.into();
    // Only a workspace that can be established refuses anything. A path
    // that does not exist yet, or an ancestor that cannot be read, falls
    // back to the table-less read, so the caller reports an error about
    // the project, not about a walk it never asked for.
    let Ok(Some((root, table))) = discover_table(&path) else {
        return Ok(crate::project::local_src::LocalSrcProject::new_access(
            path,
            nominal_path,
        ));
    };
    // `region_of` needs a canonical path to compare with the canonical root.
    let Ok(canonical) = wrapfs::canonicalize(&path) else {
        return Ok(crate::project::local_src::LocalSrcProject::new_access(
            path,
            nominal_path,
        ));
    };
    match region_of(&root, &table, &canonical) {
        PathRegion::Member(_) => Ok(crate::project::local_src::LocalSrcProject::new_member(
            path,
            nominal_path,
            std::sync::Arc::new(table),
        )),
        PathRegion::Excluded | PathRegion::Outside => Ok(
            crate::project::local_src::LocalSrcProject::new_access(path, nominal_path),
        ),
        PathRegion::Unlisted => Err(DiscoverError::Unlisted {
            project: canonical,
            root,
        }),
    }
}

/// Where a project directory sits relative to a workspace.
///
/// An excluded project is standalone, the only kind of non-member project
/// that may sit under a workspace root, while an unlisted one is a hard
/// error. Reading both table-less would make `--path <unlisted>` succeed
/// where running from inside it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRegion {
    /// The directory of `table.members[index]` itself: resolve its
    /// manifest against the table.
    Member(usize),
    /// Under an `exclude` entry: the standalone project it is.
    Excluded,
    /// Under the root and in neither list, which includes a project nested
    /// inside a member: a hard error.
    Unlisted,
    /// Not under the root.
    Outside,
}

/// [`PathRegion`] for the project directory `dir` against the workspace
/// rooted at `root`.
///
/// `dir` must be canonical when `root` is, since the two are compared.
/// [`WorkspaceTable::region`] prefix-matches, so a member is only the
/// entry itself.
pub fn region_of(root: &Utf8Path, table: &WorkspaceTable, dir: &Utf8Path) -> PathRegion {
    let relative = match relativize_path(dir, root) {
        Ok(relative) => relative,
        // A different drive or path kind, on Windows.
        Err(RelativizePathError::NoCommonPrefix { path: _, root: _ }) => {
            return PathRegion::Outside;
        }
    };
    if is_above_root(&relative) {
        return PathRegion::Outside;
    }
    match table.region(&relative) {
        Region::Member(index, member) if member.path == relative => PathRegion::Member(index),
        Region::Excluded(_) => PathRegion::Excluded,
        Region::Member(..) | Region::Unlisted => PathRegion::Unlisted,
    }
}
