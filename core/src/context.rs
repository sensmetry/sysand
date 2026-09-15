// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;

#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::LocalDirectoryEnvironment, project::local_src::LocalSrcProject,
    workspace::Workspace,
};

#[derive(Debug, Default)]
pub struct ProjectContext {
    /// Current workspace if found
    #[cfg(feature = "filesystem")]
    pub current_workspace: Option<Workspace>,
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
    /// Metadata of the current workspace/project `.sysand`. `Some` if
    /// either `current_workspace` or `current_project` is `Some` and
    /// the environment metadata file exists.
    #[cfg(feature = "filesystem")]
    pub env: Option<LocalDirectoryEnvironment>,
}

impl ProjectContext {
    #[cfg(feature = "filesystem")]
    pub fn project_root(&self) -> Option<Utf8PathBuf> {
        self.current_project
            .as_ref()
            .map(|p| p.root_path().to_owned())
    }

    #[cfg(feature = "filesystem")]
    pub fn workspace_root(&self) -> Option<Utf8PathBuf> {
        self.current_workspace
            .as_ref()
            .map(|p| p.root_path().to_owned())
    }

    #[cfg(feature = "filesystem")]
    pub fn workspace_or_project_root(&self) -> Option<Utf8PathBuf> {
        self.workspace_root().or_else(|| self.project_root())
    }
}
