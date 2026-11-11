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
    manifest::workspace_table::WorkspaceTable,
    project::local_src::LocalSrcProject,
    workspace::model_toml::{
        DiscoverError, MemberChecks, Selector, StartRegion, Workspace, member_index, start_region,
        table_of,
    },
    workspace_lock::{ExclusiveLock, LockMode, WorkspaceLock},
};

use crate::get_env;

/// How [`discover_context`] treats the workspace and the lock.
pub struct ContextOptions<'a> {
    /// Whether the command consults the `[workspace]` table: a discovery
    /// error is then fatal, and a member or the root itself runs in the
    /// workspace, with its lockfile, `.sysand` and lock.
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

/// Neither `dir` nor any of its ancestors holds a project.
#[derive(Debug, thiserror::Error)]
#[error(
    "`{dir}` is not inside a project: neither it nor any of its parent directories \
     contain a SysML v2 or KerML project"
)]
pub struct NotInProject {
    pub dir: Utf8PathBuf,
}

/// The context a command run in `cwd` acts in.
///
/// The environment, the lockfile and the lock are the workspace's for a
/// member or the root itself, and the project's otherwise (see
/// [`ProjectContext::state_root`]). `lock_mode` is given that root and
/// picks the advisory lock to take there. Configuration is read at the
/// workspace root whenever there is one.
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

    // One upward walk, reading each ancestor's manifest once, finds the
    // project and the workspace, which serves both configuration (read at
    // the workspace root) and commands with a scope.
    let ancestry = Ancestry::walk(&cwd, Seek::ProjectToo)?;
    let project_root = ancestry.project().map(Utf8Path::to_owned);
    // FIXME: assuming nominal path "." is incorrect
    let current_project = project_root
        .clone()
        .map(|root| LocalSrcProject::new_access(root, Some(".".into())));

    // The table is kept: a member's manifest cannot be read without it.
    let discovered = match table_of(ancestry) {
        Ok(found) => found.map(|(root, table)| (root, Arc::new(table))),
        Err(error) if consults_workspace => return Err(error.into()),
        // Not fatal here: failing would take down every command that does
        // not consult the workspace.
        Err(error) => {
            log::debug!("not reading configuration from a workspace root: {error}");
            None
        }
    };
    let workspace_root = discovered.as_ref().map(|(root, _)| root.clone());
    let scope = match &discovered {
        Some((root, table)) if consults_workspace => {
            Some((root.clone(), start_region(root, table, &cwd)?))
        }
        _ => None,
    };

    // A selector replaces which member the command acts on, so it is
    // applied before `current_project` is settled.
    let selected = selected_member(&cwd, discovered.as_ref(), selector)?;
    let workspace_table = discovered.filter(|_| consults_workspace);
    let current_project = match (&selected, &workspace_table) {
        (Some(dir), Some((_, table))) => Some(LocalSrcProject::new_member(
            dir.clone(),
            None,
            table.clone(),
        )),
        // "`--workspace` forces all members": the commands that have an
        // all-members path take it when there is no current project, so
        // forcing it is dropping the one the working directory implies.
        _ if selector == Selector::Workspace && workspace_root.is_some() => None,
        _ => current_project,
    };
    let project_root = selected.clone().or(project_root);

    if needs_a_member && current_project.is_none() && workspace_root.is_some() {
        return Err(DiscoverError::NeedsProject.into());
    }

    // A marker resolves against the workspace table, and only for a member,
    // so a member that inherits cannot be read without it. `member_index`,
    // not `scope`: `StartRegion::Member` prefix-matches, so it also holds
    // for a project nested inside a member, which is not one.
    let current_project = match (current_project, &workspace_table) {
        (Some(project), Some((root, table)))
            if member_index(root, table, project.root_path())?.is_some() =>
        {
            Some(LocalSrcProject::new_member(
                project.root_path().to_owned(),
                None,
                table.clone(),
            ))
        }
        (project, _) => project,
    };

    // A workspace has a single `.sysand`, at its root. Taken from `scope`,
    // not `workspace_root`: the latter is also `Some` for an `exclude`d
    // standalone project, which keeps its own state.
    let model_workspace_root = scope.as_ref().and_then(|(root, start)| {
        matches!(start, StartRegion::Member(_) | StartRegion::Workspace).then(|| root.clone())
    });

    let model_workspace_table = model_workspace_root
        .as_ref()
        .and(workspace_table)
        .map(|(_, table)| table);

    if needs_a_project && model_workspace_root.is_none() && current_project.is_none() {
        bail!(NotInProject { dir: cwd });
    }

    // `sysand.toml` is read and written at the workspace root when there
    // is one, so a broken member does not move it.
    let config_root = workspace_root
        .or_else(|| project_root.clone())
        .unwrap_or_else(|| cwd.clone());
    let mut ctx = ProjectContext {
        env: None,
        workspace_lock: None,
        cancellation,
        model_workspace_root,
        model_workspace_table,
        current_project,
        current_directory: cwd,
        config_root,
    };

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

/// The member directory `--member` names, if it names one.
///
/// `--member <path-or-name>` picks one member, matched against its
/// `members` entry as written or against its project name. The rule is
/// [`Workspace::member`], which refuses anything but a single answer.
///
/// Loading, not consulting: a project's name is in its own manifest, so a
/// name selector cannot be answered from the table. The workspace is loaded
/// only when the flag is given.
///
/// A selector with no workspace around `cwd` is an error.
///
/// With `MemberChecks::All` a member holding its own lockfile, `.sysand` or
/// `output` is refused.
fn selected_member(
    cwd: &Utf8Path,
    workspace: Option<&(Utf8PathBuf, Arc<WorkspaceTable>)>,
    selector: Selector<'_>,
) -> Result<Option<Utf8PathBuf>> {
    let Selector::Member(selector) = selector else {
        return Ok(None);
    };
    let Some((root, table)) = workspace else {
        bail!("`--member {selector}` needs a workspace, but `{cwd}` is not in one");
    };
    let workspace = Workspace::with_table(root.clone(), table.clone(), MemberChecks::All)?;
    Ok(Some(workspace.member(selector)?.dir.clone()))
}
