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
    /// The workspace with its members loaded, filled by the first load
    /// and kept for the rest of the command.
    #[cfg(feature = "filesystem")]
    pub workspace: std::sync::OnceLock<crate::workspace::model_toml::Workspace>,
    /// The parse of `current_project`'s `model.toml` that discovery made,
    /// when `current_project` is the project discovery found. It reflects
    /// the file as it was when the command started.
    #[cfg(feature = "filesystem")]
    pub current_parse: Option<crate::manifest::ParsedModelToml>,
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

    /// The workspace the command runs in, with its members loaded and
    /// every member requirement checked; `None` outside one.
    ///
    /// Loaded on the first call and kept: later calls, and
    /// [`ProjectContext::current_definition`], reuse it.
    #[cfg(feature = "filesystem")]
    pub fn load_workspace(
        &self,
    ) -> Result<
        Option<&crate::workspace::model_toml::Workspace>,
        crate::workspace::model_toml::LoadError,
    > {
        use crate::workspace::model_toml::{MemberChecks, Workspace};
        if let Some(workspace) = self.workspace.get() {
            return Ok(Some(workspace));
        }
        let Some(root) = &self.model_workspace_root else {
            return Ok(None);
        };
        let workspace = match &self.model_workspace_table {
            Some(table) => Workspace::with_table(root.clone(), table.clone(), MemberChecks::All)?,
            None => Workspace::load_with(root, MemberChecks::All)?,
        };
        Ok(Some(self.workspace.get_or_init(|| workspace)))
    }

    /// The definition of `current_project`, from a parse already in hand
    /// when there is one: the kept workspace's member, else the parse
    /// discovery made, else a read of the project.
    ///
    /// It reflects the files as they were when the command started, so it
    /// must not be used after the command has written the project.
    #[cfg(feature = "filesystem")]
    pub fn current_definition(
        &self,
    ) -> Result<Option<crate::project::ProjectDefinition>, crate::project::local_src::LocalSrcError>
    {
        use crate::project::{ProjectDefinition, ProjectFormat, ProjectRead as _};
        let Some(project) = &self.current_project else {
            return Ok(None);
        };
        if let Some(member) = self.workspace.get().and_then(|workspace| {
            workspace
                .members()
                .iter()
                .find(|m| m.dir == project.root_path())
        }) {
            return Ok(Some(ProjectDefinition::Toml(Box::new(
                member.project.clone(),
            ))));
        }
        // A directory holding both manifests is refused by the read.
        if let Some(parsed) = &self.current_parse
            && project.format()? == ProjectFormat::Toml
            && let Ok(resolved) = crate::manifest::kpar_view::resolved_of_parsed(
                crate::manifest::MODEL_TOML_FILE,
                parsed.manifest.clone(),
                project.workspace_table().map(|table| &**table),
            )
        {
            return Ok(Some(ProjectDefinition::Toml(Box::new(resolved))));
        }
        project.get_project()
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
