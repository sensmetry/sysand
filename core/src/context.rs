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
    /// In a workspace this is the workspace root, and outside one it is the
    /// project root, or the current directory when there is no project.
    #[cfg(feature = "filesystem")]
    pub config_root: Utf8PathBuf,
    /// The root of the workspace the command is running in, if it is
    /// running in one.
    ///
    /// Unlike `config_root`, there is no fallback to the project root:
    /// this is `None` outside a workspace.
    ///
    /// A solved local dependency's `src_path` is relativized against this
    /// root in the workspace lockfile.
    #[cfg(feature = "filesystem")]
    pub model_workspace_root: Option<Utf8PathBuf>,
    /// The `[workspace]` table at `model_workspace_root`, as discovery
    /// parsed it. `None` with a root means the context was built without
    /// discovery, and the table is read from the root when needed.
    #[cfg(feature = "filesystem")]
    pub model_workspace_table:
        Option<std::sync::Arc<crate::manifest::workspace_table::WorkspaceTable>>,
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
    /// The command's cancellation token, observed at every bridge the
    /// command's resolvers and storages cross and by the solver between
    /// candidates. Never cancelled by default.
    pub cancellation: crate::cancel::CancellationToken,
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
    /// The `model.toml` workspace root first, since its lockfile lives
    /// there; otherwise the project root.
    #[cfg(feature = "filesystem")]
    pub fn sources_root(&self) -> Option<Utf8PathBuf> {
        self.model_workspace_root
            .clone()
            .or_else(|| self.project_root())
    }

    /// Where `sysand-lock.toml` and `.sysand` live: the workspace root,
    /// else the project root, else the current directory.
    #[cfg(feature = "filesystem")]
    pub fn state_root(&self) -> Utf8PathBuf {
        self.sources_root()
            .unwrap_or_else(|| self.current_directory.clone())
    }

    /// The workspace the command runs in, with its members loaded; `None`
    /// outside one.
    #[cfg(feature = "filesystem")]
    pub fn load_workspace(
        &self,
        checks: crate::workspace::model_toml::MemberChecks,
    ) -> Result<
        Option<crate::workspace::model_toml::Workspace>,
        crate::workspace::model_toml::LoadError,
    > {
        use crate::workspace::model_toml::Workspace;
        let Some(root) = &self.model_workspace_root else {
            return Ok(None);
        };
        match &self.model_workspace_table {
            Some(table) => Workspace::with_table(root.clone(), table.clone(), checks).map(Some),
            None => Workspace::load_with(root, checks).map(Some),
        }
    }

    /// The `members` of the workspace the command runs in, relative to its
    /// root; none outside one.
    #[cfg(feature = "filesystem")]
    pub fn workspace_members(
        &self,
    ) -> Result<Vec<typed_path::Utf8UnixPathBuf>, crate::workspace::model_toml::DiscoverError> {
        match (&self.model_workspace_root, &self.model_workspace_table) {
            (Some(_), Some(table)) => Ok(table
                .members
                .iter()
                .map(|member| member.path.clone())
                .collect()),
            (Some(root), None) => crate::workspace::model_toml::workspace_members(root),
            (None, _) => Ok(Vec::new()),
        }
    }

    /// The projects this command's lockfile is made for, as the lockfile
    /// names them in their `editable` sources: the workspace's `members`,
    /// or else the one project at the state root, which is `.`.
    #[cfg(feature = "filesystem")]
    pub fn lock_roots(
        &self,
    ) -> Result<Vec<typed_path::Utf8UnixPathBuf>, crate::workspace::model_toml::DiscoverError> {
        match (&self.model_workspace_root, &self.model_workspace_table) {
            (Some(_), Some(table)) => Ok(table
                .members
                .iter()
                .map(|member| member.path.clone())
                .collect()),
            (Some(root), None) => crate::workspace::model_toml::lock_roots(root),
            (None, _) => Ok(vec![typed_path::Utf8UnixPathBuf::from(".")]),
        }
    }
}

#[cfg(test)]
#[path = "./context_tests.rs"]
mod tests;
