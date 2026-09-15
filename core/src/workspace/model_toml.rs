// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A workspace read from the `[workspace]` table of a `model.toml`.
//!
//! It is separate from [`crate::workspace`]'s `.workspace.json`
//! workspace, which nothing here uses.
//!
//! What a load does, beyond reading the table, is check the members --
//! workspaces.md "Member requirements" and "Stray per-member state" --
//! and resolve each member's manifest
//! against the table it belongs to. The resolution is not an
//! optimisation: [`crate::manifest::kpar_view::resolved_of_text`]
//! resolves standalone, so a member carrying `version.workspace = true`
//! is otherwise refused *for being a member*.

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;
use typed_path::Utf8Component as _;

use crate::commands::build::DEFAULT_OUTPUT_NAME;
use crate::commands::lock::DEFAULT_LOCKFILE_NAME;
use crate::env::DEFAULT_ENV_NAME;
use crate::manifest::diagnostics::{Diagnostic, Report};
use crate::manifest::project::ResolvedProject;
use crate::manifest::workspace_table::{MemberPath, Region, WorkspaceTable};
use crate::manifest::{MODEL_TOML_FILE, ModelToml, ModelTomlKind};
use crate::project::ProjectFormat;
use crate::project::local_src::{detect_format, json_manifest};
use crate::project::utils::{
    FsIoError, ProjectDeserializationError, RelativizePathError, relativize_path,
    validate_manifest, wrapfs,
};

/// Why a load failed, split by the file each half is about.
///
/// A [`Report`] cannot carry both: [`Report::sort_by_position`] orders
/// by byte offset alone and never looks at [`crate::manifest::diagnostics::Position::file`],
/// and [`ProjectDeserializationError::Invalid`] prints one path above a
/// whole report. So the diagnostics that belong to the root manifest and
/// the diagnostics that belong to a member's own manifest are kept
/// apart, each under the name of the file it points into.
///
/// Both halves are filled before the load gives up, so one bad member
/// does not hide the next.
#[derive(Debug)]
pub struct LoadFailure {
    /// Member requirements and stray per-member state, each positioned
    /// at the `members` entry that named the directory. `None` when only members' own files are bad.
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
    /// It reads, but it defines a project. workspaces.md: "the
    /// `model.toml` that defines the workspace must not also contain a
    /// `[project]` table", so a project there is not a workspace root.
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

/// Where a directory sits with respect to a workspace.
#[derive(Debug, Clone, Copy)]
pub enum Classification<'a> {
    /// Inside the member's directory, which is the member itself or a
    /// directory under it.
    Member(&'a Member),
    /// Under an `exclude`d directory: "a standalone project with its own
    /// lockfile, `.sysand` and `output`, and a workspace root there is a
    /// legitimate, separate workspace".
    Excluded(&'a MemberPath),
    /// Under the root, but in neither list. A project here is the hard
    /// error of workspaces.md "Included projects", raised on the upward
    /// walk; a directory that holds no project is simply nothing.
    Unlisted,
    /// Not under the root at all.
    Outside,
}

#[derive(Debug, Error)]
pub enum ClassifyError {
    #[error(transparent)]
    Relativize(#[from] RelativizePathError),
}

#[derive(Debug)]
pub struct Workspace {
    root: Utf8PathBuf,
    table: WorkspaceTable,
    members: Vec<Member>,
}

impl Workspace {
    /// Read the `model.toml` at `root` as a workspace, check every
    /// member and resolve each member's manifest against the table.
    ///
    /// `root` is canonicalized, so that [`Workspace::classify`] can
    /// relativize against it and so that messages name one spelling.
    ///
    /// Files are named *relative to the root* in every diagnostic --
    /// `model.toml` for the root, `app/model.toml` for a member -- since
    /// a [`crate::manifest::diagnostics::Position`] displays as
    /// `file:line:column` and three members sharing the bare constant
    /// would be indistinguishable.
    pub fn load(root: impl AsRef<Utf8Path>) -> Result<Self, LoadError> {
        let root = wrapfs::canonicalize(root.as_ref())?;
        let text = wrapfs::read_to_string(root.join(MODEL_TOML_FILE))?;
        let manifest = validate_manifest(MODEL_TOML_FILE, ModelToml::parse(MODEL_TOML_FILE, &text))
            .map_err(LoadError::Root)?;
        let table = match manifest {
            ModelToml::Workspace(table) => *table,
            ModelToml::Project(_) => return Err(LoadError::NotAWorkspace(root)),
        };

        let mut report = Report::new();
        let mut broken = Vec::new();
        let mut members = Vec::with_capacity(table.members.len());
        for path in &table.members {
            if let Some(member) = load_member(&root, path, &table, &mut report, &mut broken)? {
                members.push(member);
            }
        }

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

    /// Find the workspace a command run in `from` belongs to.
    ///
    /// The three steps are ordered, and the order matters: find the
    /// candidate, **load it**, then confirm it is not nested.
    ///
    /// [`ModelToml::peek`] answers `Workspace` for a file carrying both
    /// tables, so a member whose manifest defines both becomes a
    /// candidate. Asking the nesting question first would tell its
    /// author that "nested workspaces are not supported", about a file
    /// whose fault is that it defines both tables. The specification's
    /// "A member cannot be such a root in any case, since the two tables
    /// never share a file" is an invariant about valid files, and the
    /// walk reads invalid ones.
    pub fn discover(from: impl AsRef<Utf8Path>) -> Result<Option<Self>, DiscoverError> {
        let Some(candidate) = candidate_root(from)? else {
            return Ok(None);
        };
        let workspace = Self::load(&candidate).map_err(|e| DiscoverError::Load(Box::new(e)))?;
        confirm_root(&candidate)?;
        Ok(Some(workspace))
    }

    /// What the command started in, which is what it acts on when no
    /// selector says otherwise.
    ///
    /// [`start_region`] with the members in hand: the walk is the same
    /// one, and the index it answers with is an index into
    /// `table.members`, which a loaded workspace matches one for one
    /// (see [`Workspace::classify`]).
    pub fn start(&self, from: impl AsRef<Utf8Path>) -> Result<Selection<'_>, DiscoverError> {
        Ok(match start_region(&self.root, &self.table, from)? {
            StartRegion::Member(index) => Selection::Member(&self.members[index]),
            StartRegion::Standalone(dir) => Selection::Standalone(dir),
            StartRegion::Workspace => Selection::Workspace,
        })
    }

    /// The member `--member <path-or-name>` names.
    ///
    /// The selector matches a `members` path or a member's `name`, both
    /// literally, and a value that matches one member's path and
    /// another's name is an ambiguity error naming both, not a path match
    /// that wins by being tried first. So the rule is a union over both
    /// halves: collect every member the selector names, count a member
    /// named both ways once, and refuse anything but a single answer.
    /// There is no input whose result depends on an order.
    ///
    /// "Literally" is read as against the spelling the author wrote, so
    /// `--member ./app/` matches the entry written `./app/` and
    /// `--member app` does not.
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

    /// What `build`, `sources`, `info` and `publish` act on.
    ///
    /// [`Workspace::start`] runs first and its error is propagated
    /// before the selector is looked at: an unlisted project directory
    /// "is the hard error of Included projects, whatever the command",
    /// so a selector cannot excuse standing in one.
    pub fn select(
        &self,
        from: impl AsRef<Utf8Path>,
        selector: Selector<'_>,
    ) -> Result<Selection<'_>, DiscoverError> {
        let start = self.start(from)?;
        Ok(match selector {
            Selector::Member(selector) => Selection::Member(self.member(selector)?),
            Selector::Workspace => Selection::Workspace,
            Selector::Current => start,
        })
    }

    /// What `add`, `remove`, `include`, `exclude` and the writing `info`
    /// verbs act on.
    ///
    /// The same rule as [`Workspace::select`] except where that answers
    /// [`Selection::Workspace`]: these commands need a project, so the
    /// whole workspace is not an answer they can take, and the table
    /// makes that column an error (workspaces.md "Command scopes"). It
    /// never returns [`Selection::Workspace`].
    pub fn select_member(
        &self,
        from: impl AsRef<Utf8Path>,
        member: Option<&str>,
    ) -> Result<Selection<'_>, DiscoverError> {
        let selector = member.map_or(Selector::Current, Selector::Member);
        match self.select(from, selector)? {
            Selection::Workspace => Err(DiscoverError::NeedsProject),
            selection => Ok(selection),
        }
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn table(&self) -> &WorkspaceTable {
        &self.table
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Which of the workspace's three regions `dir` falls in.
    ///
    /// `dir` must be absolute and free of `.`/`..` components, which is
    /// what [`relativize_path`] asserts.
    ///
    /// The **most specific** entry wins -- the one whose path is the
    /// longest prefix of `dir` -- and an exact tie between the two lists
    /// goes to `members`: a directory the author listed as a member is
    /// one, and `exclude` is about what is not otherwise accounted for.
    /// So `exclude = ["vendor"]` with `members = ["vendor/a"]` puts
    /// `vendor/a/sub` inside the member and leaves `vendor/b` excluded,
    /// which is what the two lists plainly say.
    ///
    /// Nothing refuses a path written in both lists, or one list's entry
    /// nested in the other's: `member_paths` checks entries only against
    /// the others in the same list, and for `exclude` not even that. So
    /// every combination has to be decided here.
    pub fn classify(&self, dir: &Utf8Path) -> Result<Classification<'_>, ClassifyError> {
        let relative = relativize_path(dir, &self.root)?;
        if relative.components().any(|c| c.as_str() == "..") {
            return Ok(Classification::Outside);
        }

        Ok(match self.table.region(&relative) {
            // `load` pushes a diagnostic on every path where a member is
            // not produced, so a workspace that loaded has one member
            // per entry, in order.
            Region::Member(index, _) => Classification::Member(&self.members[index]),
            Region::Excluded(excluded) => Classification::Excluded(excluded),
            Region::Unlisted => Classification::Unlisted,
        })
    }
}

/// Why discovery failed.
///
/// A nesting error and an unlisted project are not diagnostics about a
/// manifest: nothing in a manifest is wrong and there is no span to
/// point at. Three of the variants *are* about a file, and arrive in two
/// shapes -- [`ModelToml::peek`] fails with a bare [`Diagnostic`] and
/// [`ModelToml::parse`] with a [`Report`] -- so both are carried as a
/// report over the file they came from.
#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// A `model.toml` on the walk that could not be read.
    #[error(transparent)]
    Manifest(ProjectDeserializationError),
    /// The candidate root is not a workspace that loads.
    #[error(transparent)]
    Load(Box<LoadError>),
    /// A workspace inside another that does not `exclude` it
    /// (workspaces.md "Config").
    #[error(
        "the workspace at `{inner}` lies inside the workspace at `{outer}`, which does not \
         `exclude` it; nested workspaces are not supported"
    )]
    Nested {
        inner: Utf8PathBuf,
        outer: Utf8PathBuf,
    },
    /// A project under the root that is in neither list (workspaces.md
    /// "Included projects").
    #[error(
        "the project at `{project}` lies under the workspace at `{root}` but is neither listed \
         in `workspace.members` nor under `workspace.exclude`; add it to one of the two"
    )]
    Unlisted {
        project: Utf8PathBuf,
        root: Utf8PathBuf,
    },
    /// [`Workspace::start`] was given a directory the workspace does not
    /// contain.
    #[error("`{dir}` is not under the workspace at `{root}`")]
    NotUnderRoot { dir: Utf8PathBuf, root: Utf8PathBuf },
    #[error(transparent)]
    Relativize(#[from] RelativizePathError),
    #[error(transparent)]
    Select(SelectError),
    /// The target is the whole workspace, and the command needs a
    /// project (workspaces.md "Command scopes").
    #[error(
        "this command needs a project; run it from a member directory or pass \
         `--member <path-or-name>`"
    )]
    NeedsProject,
}

impl From<ClassifyError> for DiscoverError {
    fn from(error: ClassifyError) -> Self {
        match error {
            ClassifyError::Relativize(error) => Self::Relativize(error),
        }
    }
}

/// Why `--member <path-or-name>` did not pick one member.
///
/// Its own type rather than a [`DiscoverError`]: selecting a member does
/// no I/O and no discovery, and a caller that only selects should not
/// have to match on variants that cannot occur.
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

impl From<SelectError> for DiscoverError {
    fn from(error: SelectError) -> Self {
        Self::Select(error)
    }
}

/// Which part of a workspace a command was asked to act on.
///
/// workspaces.md "Command scopes": `--workspace` forces all members,
/// `--member <path-or-name>` picks one, and with neither the starting
/// directory decides.
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

/// What a command acts on.
///
/// workspaces.md "Command scopes": "When run from the workspace root or
/// from a non-member directory under it that defines no project, the
/// whole workspace is the command's target; when run from a member
/// (sub)dir, only that member is."
#[derive(Debug)]
pub enum Selection<'a> {
    /// No project between the starting directory and the root, so the
    /// command targets the workspace as a whole. This is what a command
    /// run at the root gets: "the workspace root is never a member".
    Workspace,
    Member(&'a Member),
    /// A project under an `exclude`d directory: "This is the only kind
    /// of non-member project that may sit under a workspace root." The
    /// workspace was found -- the walk goes straight past such a project
    /// to the root above it -- but the command acts on the project alone,
    /// "a standalone project with its own lockfile, `.sysand` and
    /// `output`".
    Standalone(Utf8PathBuf),
}

/// Where a command run in `from` starts, answered from the table alone.
///
/// [`Workspace::start`] is this walk plus the member lookup. The split
/// exists because the two questions have different costs: this one is a
/// path comparison against [`WorkspaceTable::region`], while producing a
/// [`Member`] means [`Workspace::load`], which reads and resolves every
/// member's manifest and enforces the member requirements.
///
/// The CLI needs the cheap one first. `workspaces.md` "Stray per-member
/// state" makes a member holding a `sysand-lock.toml`, a `.sysand/` or
/// an `output/` an error. A command that writes such state into the
/// member it acts on, and asked where it starts through
/// [`Workspace::load`], would refuse the workspace its own previous run
/// created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartRegion {
    /// Inside `table.members[index]`.
    Member(usize),
    /// A project under an `exclude`d directory.
    Standalone(Utf8PathBuf),
    /// The root itself, or a directory under it that defines no project.
    Workspace,
}

/// The classification of workspaces.md "Included projects", over a
/// [`WorkspaceTable`] and a root path.
///
/// The project `from` sits in is the one [`crate::discover::discover_project`]
/// would find, so it is found with the same predicate rather than a
/// second spelling of the question. The search is bounded at the root: a
/// project above the root is a different project, which this workspace
/// has no opinion about.
///
/// `root` must be canonical -- [`candidate_root`] and [`Workspace::load`]
/// both canonicalize what they return -- because `from` is canonicalized
/// here and the two are compared.
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
        if crate::discover::is_project_file(current)? {
            let relative = relativize_path(current, root)?;
            if relative.components().any(|c| c.as_str() == "..") {
                return Err(DiscoverError::NotUnderRoot {
                    dir: current.to_owned(),
                    root: root.to_owned(),
                });
            }
            return match table.region(&relative) {
                Region::Member(index, _) => Ok(StartRegion::Member(index)),
                Region::Excluded(_) => Ok(StartRegion::Standalone(current.to_owned())),
                Region::Unlisted => Err(DiscoverError::Unlisted {
                    project: current.to_owned(),
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

/// Whether `dir` holds a `model.toml` that defines a `[workspace]`.
///
/// [`ModelToml::peek`] and nothing more, which is what it was written
/// for: reading each ancestor through [`ModelToml::parse`] would
/// validate files the walk does not care about.
///
/// A file that does not parse **ends the walk**. Treating it as "not a
/// workspace" would answer *no workspace* for a directory whose manifest
/// may well be one, and every member under it would then be read as a
/// standalone project; a wrong answer about membership is worse than an
/// error. A file with neither table is walked past -- it is not a
/// workspace root, whatever else is wrong with it.
fn is_workspace_file(dir: &Utf8Path) -> Result<bool, DiscoverError> {
    let file = dir.join(MODEL_TOML_FILE);
    if !wrapfs::is_file(&file)? {
        return Ok(false);
    }
    let text = wrapfs::read_to_string(&file)?;
    match ModelToml::peek(file.as_str(), &text) {
        Ok(Some(ModelTomlKind::Workspace)) => Ok(true),
        Ok(_) => Ok(false),
        Err(diagnostic) => Err(DiscoverError::Manifest(
            ProjectDeserializationError::invalid(&file, Report::from_iter([diagnostic])),
        )),
    }
}

/// The `[workspace]` table `dir` defines.
///
/// Parsed, not loaded. [`Workspace::load`] also checks that each member
/// directory exists, holds a `model.toml` project, is free of stray
/// state and has a manifest that reads -- none of which bears on whether
/// this workspace excludes some directory, and a command inside a
/// separate workspace must not fail because a neighbour's member is
/// missing.
///
/// Parsing is not free of the outer's own mistakes: `parse` runs
/// [`WorkspaceTable::validate`], whose report is all-or-nothing, so an
/// outer whose `members` or `exclude` holds a path that does not
/// normalize does not parse. That is accepted: a table whose `exclude`
/// does not parse cannot answer the question being asked of it.
fn workspace_table_at(dir: &Utf8Path) -> Result<WorkspaceTable, DiscoverError> {
    let file = dir.join(MODEL_TOML_FILE);
    let text = wrapfs::read_to_string(&file)?;
    let manifest = validate_manifest(&file, ModelToml::parse(file.as_str(), &text))
        .map_err(DiscoverError::Manifest)?;
    manifest.into_workspace(file.as_str()).map_err(|problem| {
        DiscoverError::Manifest(ProjectDeserializationError::invalid(
            &file,
            Report::from_iter([problem]),
        ))
    })
}

/// Pass 1: the first ancestor of `from` whose `model.toml` defines a
/// `[workspace]`.
///
/// workspaces.md: "The workspace is discovered by walking up from cwd
/// until a `model.toml` with a `[workspace]` table is encountered; that
/// folder is the candidate workspace root."
///
/// A *candidate*, because the walk is not finished: see [`confirm_root`].
/// And because a file carrying both tables peeks as a workspace, a
/// candidate is not yet known to be a readable one either --
/// [`Workspace::discover`] reads it before ruling on nesting.
///
/// `from` is canonicalized once. [`relativize_path`] asserts absolute
/// and dot-free paths with a plain `assert!`, and every ancestor of a
/// canonical path is canonical.
pub fn candidate_root(from: impl AsRef<Utf8Path>) -> Result<Option<Utf8PathBuf>, DiscoverError> {
    let from = wrapfs::canonicalize(from.as_ref())?;
    let mut current: &Utf8Path = &from;
    loop {
        if is_workspace_file(current)? {
            return Ok(Some(current.to_owned()));
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => return Ok(None),
        }
    }
}

/// The workspace `from` belongs to, as its `[workspace]` table.
///
/// [`Workspace::discover`] loads: it parses and resolves every member,
/// and refuses the workspace if any is broken. This does not: creating
/// a new project, for one, cannot be blocked by an unrelated member's
/// mistake, or the one command that could finish a half-built workspace
/// is the one that refuses to run in it.
///
/// The three steps are [`candidate_root`], the parse, and
/// [`confirm_root`], in that order, so that a file carrying both tables
/// fails as *that* rather than as nesting (see [`Workspace::discover`]). A bad
/// candidate therefore arrives as [`DiscoverError::Manifest`], where
/// [`Workspace::discover`] would report [`DiscoverError::Load`].
pub fn discover_table(
    from: impl AsRef<Utf8Path>,
) -> Result<Option<(Utf8PathBuf, WorkspaceTable)>, DiscoverError> {
    let Some(candidate) = candidate_root(from)? else {
        return Ok(None);
    };
    let table = workspace_table_at(&candidate)?;
    confirm_root(&candidate)?;
    Ok(Some((candidate, table)))
}

/// Pass 2: refuse a candidate nested in another workspace.
///
/// workspaces.md: "The walk does not stop there: it continues to the
/// filesystem root (there is no `.git` bound), and a second
/// `[workspace]` table found above the first means the candidate is
/// itself inside another workspace. That is allowed in exactly one case:
/// the outer workspace lists the candidate's directory, or a directory
/// above it, in `exclude`."
///
/// "Then the candidate is confirmed as the root and the walk stops;
/// whether the outer workspace is itself nested is checked when a
/// command runs inside it." So the first outer settles the question
/// either way, and a third workspace above it is not this command's
/// problem.
pub fn confirm_root(candidate: impl AsRef<Utf8Path>) -> Result<(), DiscoverError> {
    let candidate = wrapfs::canonicalize(candidate.as_ref())?;
    let mut current = candidate.parent().map(Utf8Path::to_owned);
    while let Some(dir) = current {
        if is_workspace_file(&dir)? {
            let table = workspace_table_at(&dir)?;
            let relative = relativize_path(&candidate, &dir)?;
            return if table.excluding(&relative).is_some() {
                Ok(())
            } else {
                Err(DiscoverError::Nested {
                    inner: candidate,
                    outer: dir,
                })
            };
        }
        current = dir.parent().map(Utf8Path::to_owned);
    }
    Ok(())
}

/// Whether `path` is there, and if so whether it is a directory.
///
/// `wrapfs::is_dir` alone cannot tell an absent entry from a file, and
/// the two are different mistakes: a `members` entry naming nothing is a
/// typo or a directory yet to be created, one naming a file is not.
fn entry_kind(path: &Utf8Path) -> Result<Option<bool>, Box<FsIoError>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.is_dir())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Box::new(FsIoError::IsFile(path.into(), err))),
    }
}

/// workspaces.md "Member requirements", and the read that answers them.
///
/// [`detect_format`] tests file existence, which settles the json, the
/// ambiguous and the empty directory. It cannot settle the fourth case:
/// a `model.toml` holding a `[workspace]` table, one that is not TOML,
/// and one defining neither table all come back [`ProjectFormat::Toml`].
/// Since the member has to be parsed anyway -- its manifest is resolved
/// against the table -- the parse is the check, and
/// [`ModelToml::peek`] is left to the upward walk, where parsing every
/// ancestor would validate files nobody asked about.
fn load_member(
    root: &Utf8Path,
    path: &MemberPath,
    table: &WorkspaceTable,
    report: &mut Report,
    broken: &mut Vec<ProjectDeserializationError>,
) -> Result<Option<Member>, Box<FsIoError>> {
    let raw = &path.raw;
    let dir = root.join(path.path.as_str());

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

    // Run whatever the directory being present makes answerable, so that
    // a member with both a stray lockfile and the wrong manifest format
    // reports both.
    stray_state(root, path, &dir, report)?;

    match detect_format(&dir)? {
        ProjectFormat::Json => {
            let json = json_manifest(&dir)?.expect("`Json` means one of the two is there");
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` uses `{json}`; a workspace member must use the `model.toml` \
                     project definition (`sysand migrate {raw}`)"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Both => {
            let json = json_manifest(&dir)?.expect("`Both` means one of the two is there");
            // The clause is `LocalSrcError::AmbiguousFormat`'s, written
            // out rather than stringified from it: this message names
            // the member, and folding a typed error into a diagnostic is
            // what `LoadFailure` exists to avoid. A test holds the two
            // together.
            report.push(
                Diagnostic::error(format!(
                    "member `{raw}` has both `model.toml` and `{json}`; it is unclear whether it \
                     is a non-zipped KPAR or a development project"
                ))
                .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Neither => {
            report.push(
                Diagnostic::error(format!("member `{raw}` defines no project"))
                    .at_opt(path.at.clone()),
            );
            Ok(None)
        }
        ProjectFormat::Toml => {
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

/// workspaces.md "Stray per-member state": "three
/// existence tests, no content is read".
///
/// The message names the *root*, since that is where the state belongs
/// and saying so is the whole content of the rule. Excluded directories
/// are not checked: "Standalone projects under an `exclude`d directory
/// keep their state legitimately; the check applies to members only."
fn stray_state(
    root: &Utf8Path,
    path: &MemberPath,
    dir: &Utf8Path,
    report: &mut Report,
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
