// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;

#[cfg(feature = "filesystem")]
use crate::{env::local_directory::LocalDirectoryEnvironment, project::local_src::LocalSrcProject};

#[derive(Debug, Default)]
pub struct ProjectContext {
    /// Current project if found
    #[cfg(feature = "filesystem")]
    pub current_project: Option<LocalSrcProject>,
    /// Path to current directory
    #[cfg(feature = "filesystem")]
    pub current_directory: Utf8PathBuf,
    /// Where `sysand.toml` is read and written, and what a relative
    /// override path is relative to.
    ///
    /// workspaces.md "Source overrides": "a project-local one must be at
    /// the workspace root: overrides apply workspace-wide ... as a
    /// member cannot override sources for itself alone". So in a
    /// workspace this is the workspace root, and outside one it is the
    /// project root -- or the current directory when there is no
    /// project.
    ///
    /// The three uses have to agree. Reading the config at the root
    /// while a relative override path still resolved against the member,
    /// or while `add` still wrote into the member, would record an
    /// override that the next command ignores and warns about.
    #[cfg(feature = "filesystem")]
    pub config_root: Utf8PathBuf,
    /// The root of the workspace the command is running in, if it is
    /// running in one.
    ///
    /// Distinct from `config_root`, which answers the same question for
    /// configuration and falls back to the project root: this is `None`
    /// outside a workspace, because "relative to the workspace root" has
    /// no fallback meaning.
    ///
    /// A solved local dependency has no nominal path, so its `src_path`
    /// is relativized against a root, and a lockfile at the workspace
    /// root needs that root -- `workspaces.md` "Locking" records "every project of the
    /// solved graph ... members included" in one file, so every path in
    /// it means the same thing.
    #[cfg(feature = "filesystem")]
    pub model_workspace_root: Option<Utf8PathBuf>,
    /// Metadata of the current workspace/project `.sysand`. `Some` if
    /// the workspace root or `current_project` is known and the
    /// environment metadata file exists.
    #[cfg(feature = "filesystem")]
    pub env: Option<LocalDirectoryEnvironment>,
    /// The advisory lock the command holds on the workspace, acquired
    /// once at command top and handed to every environment the command
    /// writes; `None` for readers.
    #[cfg(feature = "filesystem")]
    pub workspace_lock: Option<std::sync::Arc<crate::workspace_lock::WorkspaceLock>>,
}

impl ProjectContext {
    #[cfg(feature = "filesystem")]
    pub fn project_root(&self) -> Option<Utf8PathBuf> {
        self.current_project
            .as_ref()
            .map(|p| p.root_path().to_owned())
    }

    /// The root a `src_path` in this command's lockfile is relative to.
    ///
    /// The `model.toml` workspace root first: its lockfile lives there,
    /// and a path in it that meant "relative to whichever member
    /// happened to run `lock`" would mean different things in one file.
    #[cfg(feature = "filesystem")]
    pub fn sources_root(&self) -> Option<Utf8PathBuf> {
        self.model_workspace_root
            .clone()
            .or_else(|| self.project_root())
    }
}

#[cfg(test)]
#[path = "./context_tests.rs"]
mod tests;
