// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Where a command runs: the project, the workspace around it, and where
//! its lockfile, `.sysand` and configuration live.

use camino::{Utf8Path, Utf8PathBuf};
use sysand_core::{
    context_discovery::{self, ContextError},
    workspace::model_toml::Selector,
    workspace_lock::LockMode,
};

use crate::Failure;

pub use sysand_core::context_discovery::{
    ContextOptions, Discovered, NotInProject, config_root_of,
};

/// [`context_discovery::discover_context`], with the hint the
/// environment's error calls for.
pub fn discover_context(
    cwd: Utf8PathBuf,
    options: ContextOptions<'_>,
    lock_mode: impl FnOnce(&Utf8Path) -> Option<LockMode>,
) -> Result<Discovered, Failure> {
    context_discovery::discover_context(cwd, options, lock_mode).map_err(failure)
}

fn failure(error: ContextError) -> Failure {
    match error {
        ContextError::Environment(error) => Failure::environment(error),
        other => other.into(),
    }
}

/// A directory a command names is not a directory.
#[derive(Debug, thiserror::Error)]
#[error("`{dir}` is not a directory")]
pub struct NotADirectory {
    pub dir: Utf8PathBuf,
}

/// Where a command's context is built from.
#[derive(Debug)]
pub enum Start<'a> {
    /// The working directory.
    Cwd,
    /// A directory the command names, which it acts in. `hint` is added to
    /// the error when it is not a directory.
    Dir {
        dir: &'a Utf8Path,
        hint: Option<&'static str>,
    },
    /// A directory the command names and creates when it is missing.
    NewDir(&'a Utf8Path),
}

/// What a command needs of its context, for [`discover_context`].
#[derive(Debug)]
pub struct Needs<'a> {
    pub start: Start<'a>,
    pub consults_workspace: bool,
    pub selector: Selector<'a>,
    pub needs_a_member: bool,
    pub needs_a_project: bool,
}

impl<'a> Needs<'a> {
    /// A command that acts in the working directory's workspace; one that
    /// acts on a member also needs a project.
    pub fn acting(selector: Selector<'a>, needs_a_member: bool) -> Self {
        Self {
            start: Start::Cwd,
            consults_workspace: true,
            selector,
            needs_a_member,
            needs_a_project: needs_a_member,
        }
    }
}
