// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Where a command runs: the project, the workspace around it, and where
//! its lockfile, `.sysand` and configuration live.

use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use sysand_core::{
    cancel::CancellationToken,
    context::ProjectContext,
    discover::{Ancestry, Seek},
    manifest::{ParsedModelToml, workspace_table::WorkspaceTable},
    project::local_src::LocalSrcProject,
    workspace::model_toml::{
        DiscoverError, MemberChecks, Selector, StartRegion, Workspace, start_region, table_of,
    },
    workspace_lock::{ExclusiveLock, LockMode, WorkspaceLock},
};

use crate::{cli, get_env};

/// How [`discover_context`] treats the workspace and the lock.
pub struct ContextOptions<'a> {
    /// Whether the command acts in the workspace: a discovery error is
    /// then fatal, and a member or the root itself runs in the workspace,
    /// loaded in full, with its lockfile, `.sysand` and lock.
    pub consults_workspace: bool,
    /// Which member the command acts on.
    pub selector: Selector<'a>,
    /// The command acts on one project, so in a workspace with no member
    /// to act on it fails with [`DiscoverError::NeedsProject`].
    pub needs_a_member: bool,
    /// Outside both a project and a workspace the command fails with
    /// [`NotInProject`], before anything is locked or created.
    pub needs_a_project: bool,
    /// How long to wait for a held lock; `None` fails at once.
    pub lock_wait: Option<Duration>,
    pub cancellation: CancellationToken,
}

/// What [`discover_context`] found.
pub struct Discovered {
    pub ctx: ProjectContext,
    /// The member directory the selector named, if it named one.
    pub selected: Option<Utf8PathBuf>,
    /// The selected member's directory, or the project's the command
    /// started in.
    pub project_root: Option<Utf8PathBuf>,
    /// The workspace root and where under it the command started, for a
    /// command that consults the workspace.
    pub scope: Option<(Utf8PathBuf, StartRegion)>,
}

/// A directory a command names is not a directory.
#[derive(Debug, thiserror::Error)]
#[error("`{dir}` is not a directory")]
pub struct NotADirectory {
    pub dir: Utf8PathBuf,
}

/// Neither `dir` nor any of its ancestors holds a project.
#[derive(Debug, thiserror::Error)]
#[error(
    "`{dir}` is not inside a project: neither it nor any of its parent directories \
     contain a SysML v2 or KerML project"
)]
pub struct NotInProject {
    pub dir: Utf8PathBuf,
}

/// The context a command run in `cwd` acts in, read in full.
///
/// The environment, the lockfile and the lock are the workspace's for a
/// member or the root itself, and the project's otherwise (see
/// [`ProjectContext::state_root`]). `lock_mode` is given that root and
/// picks the advisory lock to take there. Configuration is read at the
/// workspace root whenever there is one.
///
/// A workspace is loaded with every member, and the current project's
/// manifest is parsed (or taken from its member): a member or a manifest
/// that does not load fails the construction.
pub fn discover_context(
    cwd: Utf8PathBuf,
    options: ContextOptions<'_>,
    lock_mode: impl FnOnce(&Utf8Path) -> Option<LockMode>,
) -> Result<Discovered> {
    let ContextOptions {
        consults_workspace,
        selector,
        needs_a_member,
        needs_a_project,
        lock_wait,
        cancellation,
    } = options;

    let Walk {
        project_root,
        project_parse,
        workspace: discovered,
    } = Walk::of(&cwd, consults_workspace)?;
    let workspace_root = discovered.as_ref().map(|(root, _)| root.clone());
    let scope = match &discovered {
        Some((root, table)) if consults_workspace => {
            Some((root.clone(), start_region(root, table, &cwd)?))
        }
        _ => None,
    };

    // The workspace is the command's own when it runs in a member, at the
    // root or on a member the selector names. A standalone project under
    // an `exclude`d directory keeps its own state.
    let in_workspace = scope
        .as_ref()
        .is_some_and(|(_, start)| matches!(start, StartRegion::Member(_) | StartRegion::Workspace))
        || (consults_workspace && matches!(selector, Selector::Member(_) | Selector::Workspace));
    let workspace = match &discovered {
        Some((root, table)) if in_workspace => Some(Workspace::with_table(
            root.clone(),
            table.clone(),
            MemberChecks::All,
        )?),
        _ => None,
    };

    // A selector replaces which member the command acts on, so it is
    // applied before `current_project` is settled.
    let selected = match selector {
        Selector::Member(selector) => {
            let Some(workspace) = &workspace else {
                bail!("`--member {selector}` needs a workspace, but `{cwd}` is not in one");
            };
            Some(workspace.member(selector)?.dir.clone())
        }
        Selector::Current | Selector::Workspace => None,
    };
    // The root of the project the command acts on.
    let acting_root = match (&selected, &workspace) {
        (Some(dir), Some(_)) => Some(dir.clone()),
        // "`--workspace` forces all members": the commands that have an
        // all-members path take it when there is no current project, so
        // forcing it is dropping the one the working directory implies.
        _ if selector == Selector::Workspace && workspace.is_some() => None,
        _ => project_root.clone(),
    };
    // The walk's parse belongs to the project the walk found, which a
    // selector replaces.
    let project_parse = project_parse.filter(|_| {
        selected.is_none()
            && acting_root
                .as_ref()
                .is_some_and(|root| project_root.as_ref() == Some(root))
    });
    let project_root = selected.clone().or(project_root);

    if needs_a_member && acting_root.is_none() && workspace_root.is_some() {
        return Err(DiscoverError::NeedsProject.into());
    }

    // A marker resolves against the workspace table, and only for a member,
    // so a member that inherits cannot be read without it.
    let member = match (&acting_root, &workspace) {
        (Some(root), Some(workspace)) => workspace.member_index_of(root)?,
        _ => None,
    };
    let current_project = acting_root.map(|root| match (member, &workspace) {
        (Some(_), Some(workspace)) => {
            LocalSrcProject::new_member(root, None, workspace.table().clone())
        }
        // FIXME: assuming nominal path "." is incorrect
        _ => LocalSrcProject::new_access(root, Some(".".into())),
    });

    if needs_a_project && workspace.is_none() && current_project.is_none() {
        bail!(NotInProject { dir: cwd });
    }

    // A member's manifest was parsed with the workspace; any other
    // project's is the walk's parse, or the json pair.
    let current_definition = match (&current_project, &workspace, member) {
        (Some(_), Some(workspace), Some(index)) => {
            Some(sysand_core::project::ProjectDefinition::Toml(Box::new(
                workspace.members()[index].project.clone(),
            )))
        }
        (Some(project), _, _) => project.get_project_parsed(project_parse)?,
        (None, _, _) => None,
    };

    // `sysand.toml` is read and written at the workspace root when there
    // is one, so a broken member does not move it.
    let config_root = workspace_root
        .or_else(|| project_root.clone())
        .unwrap_or_else(|| cwd.clone());
    let mut ctx = ProjectContext::new(workspace, current_project, current_definition);
    ctx.cancellation = cancellation;
    ctx.current_directory = cwd;
    ctx.config_root = config_root;

    // A mutating command holds the workspace's advisory lock for the whole
    // invocation, taken once before the environment is read and handed to
    // every environment it writes.
    let state_root = ctx.state_root();
    if let Some(mode) = lock_mode(&state_root) {
        ctx.workspace_lock = Some(Arc::new(WorkspaceLock::acquire(
            &state_root,
            mode,
            lock_wait,
        )?));
    }
    ctx.env = get_env(&state_root)?;
    // The environment writes under an exclusive lock only.
    if let (Some(env), Some(lock)) = (
        &mut ctx.env,
        ctx.workspace_lock.clone().and_then(ExclusiveLock::new),
    ) {
        env.hold(lock)?;
    }

    Ok(Discovered {
        ctx,
        selected,
        project_root,
        scope,
    })
}

/// Where a command's context is built from.
pub enum Start<'a> {
    /// The working directory.
    Cwd,
    /// A directory the command names, which it acts in.
    Dir(&'a Utf8Path),
    /// A directory the command names and creates when it is missing.
    NewDir(&'a Utf8Path),
}

/// What a command needs of its context, for [`discover_context`].
pub struct Needs<'a> {
    pub start: Start<'a>,
    pub consults_workspace: bool,
    pub selector: Selector<'a>,
    pub needs_a_member: bool,
    pub needs_a_project: bool,
}

/// What `command` needs of its context, or `None` when it has none: it
/// acts on nothing in the current project or workspace, and needs only the
/// working directory.
///
/// A command acts in the workspace, and so loads it, unless it is
/// `migrate` (an unlisted json project under a workspace root is a valid
/// target, and a json member is migrated before the workspace can load) or
/// `clone` (which builds the context of the project it creates).
pub fn context_needs(command: &cli::Command) -> Option<Needs<'_>> {
    use cli::Command;
    /// A command that acts on one member also needs a project.
    fn acting(selector: Selector<'_>, needs_a_member: bool) -> Needs<'_> {
        Needs {
            start: Start::Cwd,
            consults_workspace: true,
            selector,
            needs_a_member,
            needs_a_project: needs_a_member,
        }
    }
    match command {
        Command::Build {
            selector,
            path: _,
            compression: _,
            allow_path_usage: _,
            keep_index: _,
        }
        | Command::Sources {
            selector,
            sources_opts: _,
        } => Some(Needs {
            needs_a_project: true,
            ..acting(selector.selector(), false)
        }),
        // An explicit archive path needs no project.
        Command::Publish {
            selector,
            path: _,
            index: _,
            trusted_publishing: _,
        } => Some(acting(selector.selector(), false)),
        Command::PrintRoot { selector } => Some(Needs {
            needs_a_project: true,
            ..acting(selector.selector(), false)
        }),
        // `--kpar-path` names its own project; `--dir` starts the context
        // there; an identifier or an IRI is looked up in the environment too.
        Command::Info {
            selector,
            locator,
            get: _,
            resolution_opts: _,
        } => locator.kpar_path.is_none().then(|| Needs {
            start: locator.dir.as_deref().map_or(Start::Cwd, Start::Dir),
            ..acting(selector.selector(), false)
        }),
        Command::Add {
            selector,
            locator: _,
            version_constraint: _,
            sync: _,
            resolution_opts: _,
            source_opts: _,
        }
        | Command::Remove {
            selector,
            locator: _,
            sync: _,
            resolution_opts: _,
        }
        | Command::Include {
            selector,
            paths: _,
            compute_checksum: _,
            no_index_symbols: _,
        }
        | Command::Exclude { selector, paths: _ } => Some(acting(selector.selector(), true)),
        Command::Edit {
            dir,
            selector,
            edits: _,
        } => Some(Needs {
            start: dir.as_deref().map_or(Start::Cwd, Start::Dir),
            consults_workspace: true,
            selector: selector.selector(),
            needs_a_member: true,
            needs_a_project: true,
        }),
        Command::Lock { resolution_opts: _ }
        | Command::Sync {
            resolution_opts: _,
            no_prune: _,
            verify: _,
            repair: _,
        }
        | Command::Versions {
            iri: _,
            resolution_opts: _,
        }
        | Command::Env { command: _ } => Some(acting(Selector::Current, false)),
        Command::Init {
            path,
            name: _,
            publisher: _,
            version: _,
            license: _,
            kind: _,
            metamodel: _,
            metamodel_release: _,
        } => Some(Needs {
            start: path.as_deref().map_or(Start::Cwd, Start::NewDir),
            consults_workspace: true,
            selector: Selector::Current,
            needs_a_member: false,
            needs_a_project: false,
        }),
        Command::Migrate {
            path,
            publisher: _,
            metamodel: _,
            allow_dirty: _,
            allow_no_vcs: _,
            resolution_opts: _,
        } => Some(Needs {
            start: path.as_deref().map_or(Start::Cwd, Start::Dir),
            consults_workspace: false,
            selector: Selector::Current,
            needs_a_member: false,
            needs_a_project: false,
        }),
        Command::Clone {
            locator: _,
            target: _,
            version_constraint: _,
            no_deps: _,
            resolution_opts: _,
        }
        | Command::Index { command: _ }
        | Command::Auth { command: _ }
        | Command::Experimental {} => None,
    }
}

/// Where `sysand.toml` is read for a command that has no context: at the
/// workspace root when `cwd` is in one, else at the project root, else in
/// `cwd`.
pub fn config_root_of(cwd: &Utf8Path) -> Result<Utf8PathBuf> {
    let walk = Walk::of(cwd, false)?;
    Ok(walk
        .workspace
        .map(|(root, _)| root)
        .or(walk.project_root)
        .unwrap_or_else(|| cwd.to_owned()))
}

/// What the one upward walk from the working directory found.
struct Walk {
    /// The nearest project, as the walk spells it.
    project_root: Option<Utf8PathBuf>,
    /// The parse of that project's `model.toml`, if it has one.
    project_parse: Option<ParsedModelToml>,
    /// The workspace root and its table, kept because a member's manifest
    /// cannot be read without it.
    workspace: Option<(Utf8PathBuf, Arc<WorkspaceTable>)>,
}

impl Walk {
    /// Each ancestor's manifest is read once. A workspace that cannot be
    /// established is an error only when `consults_workspace`: failing
    /// otherwise would take down every command that does not act in it.
    fn of(cwd: &Utf8Path, consults_workspace: bool) -> Result<Self> {
        let ancestry = Ancestry::walk(cwd, Seek::ProjectToo)?;
        let project_root = ancestry.project().map(Utf8Path::to_owned);
        let project_parse = ancestry.project_model_toml().cloned();
        let workspace = match table_of(ancestry) {
            Ok(found) => found.map(|(root, table)| (root, Arc::new(table))),
            Err(error) if consults_workspace => return Err(error.into()),
            Err(error) => {
                log::debug!("not reading configuration from a workspace root: {error}");
                None
            }
        };
        Ok(Self {
            project_root,
            project_parse,
            workspace,
        })
    }
}
