// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use std::sync::Arc;

#[cfg(feature = "filesystem")]
use camino::{Utf8Path, Utf8PathBuf};
#[cfg(feature = "filesystem")]
use thiserror::Error;
#[cfg(feature = "filesystem")]
use typed_path::Utf8UnixPathBuf;

use crate::cancel::CancellationToken;
#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::LocalDirectoryEnvironment,
    manifest::{MODEL_TOML_FILE, edit::DocumentMut, kpar_view},
    paths::{RelativizePathError, relativize_path},
    project::{
        ManifestError, ManifestWriteError, ProjectDefinition, ProjectMut as _, ProjectRead as _,
        local_src::{LocalSrcError, LocalSrcProject},
        utils::{FsIoError, ProjectDeserializationError},
    },
    workspace::model_toml::{LoadError, Workspace},
    workspace_lock::WorkspaceLock,
};

/// A snapshot of the project and the workspace a command runs in, read in
/// full when the context is built.
///
/// The files are not read again: the writes a command makes to the
/// current project go through [`ProjectContext::edit_project`] and
/// [`ProjectContext::edit_manifest`], which bring the snapshot up to date.
#[derive(Debug, Default)]
pub struct ProjectContext {
    /// Current project if found
    #[cfg(feature = "filesystem")]
    current_project: Option<LocalSrcProject>,
    /// The manifest of `current_project`, as read. `Some` whenever there
    /// is a current project that has one.
    #[cfg(feature = "filesystem")]
    current_definition: Option<ProjectDefinition>,
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
    /// The workspace the command is running in, with every member loaded.
    ///
    /// There is no fallback to the project: this is `None` outside a
    /// workspace. A solved local dependency's `src_path` is relativized
    /// against its root in the workspace lockfile.
    #[cfg(feature = "filesystem")]
    workspace: Option<Workspace>,
    /// Metadata of the current workspace/project `.sysand`. `Some` if
    /// the workspace root or `current_project` is known and the
    /// environment metadata file exists.
    #[cfg(feature = "filesystem")]
    pub env: Option<LocalDirectoryEnvironment>,
    /// The advisory lock the command holds on the workspace, acquired
    /// once at command top and handed to every environment the command
    /// writes; `None` for readers.
    #[cfg(feature = "filesystem")]
    pub workspace_lock: Option<Arc<WorkspaceLock>>,
    /// The command's cancellation token, observed at every bridge the
    /// command's resolvers and storages cross and by the solver between
    /// candidates. Never cancelled by default.
    pub cancellation: CancellationToken,
}

/// Why a snapshot could not be brought up to date after a write.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum ReloadError {
    #[error(transparent)]
    Project(#[from] LocalSrcError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// The workspace no longer holds together with the written manifest.
    #[error(transparent)]
    Workspace(#[from] Box<LoadError>),
    /// A member that is not a `model.toml` project.
    #[error("the member at `{0}` is not a `model.toml` project")]
    NotAModelToml(Utf8PathBuf),
}

/// Why a write to the current project's manifest failed.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum EditError<E> {
    #[error("there is no current project")]
    NoProject,
    /// The edit itself failed.
    #[error(transparent)]
    Edit(E),
    /// The edit was written, and the files no longer pass the checks the
    /// context makes.
    #[error(transparent)]
    Reload(ReloadError),
}

/// The failures of [`ProjectContext::edit_manifest`] that are about the
/// manifest document.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum ManifestStep<E> {
    #[error(transparent)]
    Read(ManifestError<LocalSrcError>),
    #[error(transparent)]
    Edit(E),
    /// The edited manifest does not resolve, so it is not written.
    #[error(transparent)]
    Invalid(ProjectDeserializationError),
    #[error(transparent)]
    Write(ManifestWriteError<LocalSrcError>),
}

#[cfg(feature = "filesystem")]
impl ProjectContext {
    /// A context holding the given snapshot; every other field starts at
    /// its default.
    ///
    /// `current_definition` is the manifest of `current_project`, and
    /// `workspace` has the current project among its members when it is
    /// one.
    pub fn new(
        workspace: Option<Workspace>,
        current_project: Option<LocalSrcProject>,
        current_definition: Option<ProjectDefinition>,
    ) -> Self {
        Self {
            current_project,
            current_definition,
            workspace,
            ..Self::default()
        }
    }

    /// The current project, if there is one.
    pub fn current_project(&self) -> Option<&LocalSrcProject> {
        self.current_project.as_ref()
    }

    /// The manifest of the current project, as of the last read or write.
    pub fn current_definition(&self) -> Option<&ProjectDefinition> {
        self.current_definition.as_ref()
    }

    /// The workspace the command runs in, with every member loaded.
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    pub fn project_root(&self) -> Option<Utf8PathBuf> {
        self.current_project
            .as_ref()
            .map(|p| p.root_path().to_owned())
    }

    /// The root of the workspace the command runs in, if it is running in
    /// one.
    pub fn model_workspace_root(&self) -> Option<&Utf8Path> {
        self.workspace.as_ref().map(Workspace::root)
    }

    /// The root a `src_path` in this command's lockfile is relative to.
    ///
    /// The `model.toml` workspace root first, since its lockfile lives
    /// there; otherwise the project root.
    pub fn sources_root(&self) -> Option<Utf8PathBuf> {
        self.model_workspace_root()
            .map(Utf8Path::to_owned)
            .or_else(|| self.project_root())
    }

    /// The path a local project at `path` is written under in this
    /// command's lockfile: `nominal` when given, else `path` relative to
    /// [`Self::sources_root`].
    ///
    /// # Panics
    ///
    /// If there is neither a `nominal` path nor a sources root.
    pub fn lockfile_path(
        &self,
        nominal: Option<&Utf8UnixPathBuf>,
        path: &Utf8Path,
    ) -> Result<Utf8UnixPathBuf, RelativizePathError> {
        if let Some(nominal) = nominal {
            return Ok(nominal.clone());
        }
        let root = self
            .sources_root()
            .expect("a local project without a nominal path has no lockfile path");
        relativize_path(path, root)
    }

    /// Where `sysand-lock.toml` and `.sysand` live: the workspace root,
    /// else the project root, else the current directory.
    pub fn state_root(&self) -> Utf8PathBuf {
        self.sources_root()
            .unwrap_or_else(|| self.current_directory.clone())
    }

    /// The `members` of the workspace the command runs in, relative to its
    /// root; none outside one.
    pub fn workspace_members(&self) -> Vec<Utf8UnixPathBuf> {
        self.workspace.as_ref().map_or_else(Vec::new, |workspace| {
            workspace
                .table()
                .members
                .iter()
                .map(|member| member.path.clone())
                .collect()
        })
    }

    /// The projects this command's lockfile is made for, as the lockfile
    /// names them in their `editable` sources: the workspace's `members`,
    /// or else the one project at the state root, which is `.`.
    pub fn lock_roots(&self) -> Vec<Utf8UnixPathBuf> {
        match &self.workspace {
            Some(_) => self.workspace_members(),
            None => vec![Utf8UnixPathBuf::from(".")],
        }
    }

    /// Let `edit` change the current project, then read it again, so the
    /// snapshot matches what `edit` wrote. In a workspace the checks that
    /// span members run again over the members in hand.
    ///
    /// The snapshot is read again also when `edit` fails, as it may have
    /// written before failing; the error of `edit` is the one returned.
    pub fn edit_project<T, E>(
        &mut self,
        edit: impl FnOnce(&mut LocalSrcProject) -> Result<T, E>,
    ) -> Result<T, EditError<E>> {
        let Some(project) = self.current_project.as_mut() else {
            return Err(EditError::NoProject);
        };
        let edited = edit(project);
        let reloaded = self.reload_current();
        match edited {
            Err(error) => Err(EditError::Edit(error)),
            Ok(value) => reloaded.map(|()| value).map_err(EditError::Reload),
        }
    }

    /// [`ProjectContext::edit_project`] for a `model.toml` project:
    /// loads the manifest document, applies `edit` to it, and writes it,
    /// refusing a manifest that does not resolve.
    pub fn edit_manifest<T, E>(
        &mut self,
        edit: impl FnOnce(&mut DocumentMut) -> Result<T, E>,
    ) -> Result<T, EditError<ManifestStep<E>>> {
        self.edit_project(|project| {
            let mut document = project.manifest_document().map_err(ManifestStep::Read)?;
            let value = edit(&mut document).map_err(ManifestStep::Edit)?;
            kpar_view::resolved_of_text_in(
                MODEL_TOML_FILE,
                &document.to_string(),
                project.workspace_table().map(|table| &**table),
            )
            .map_err(ManifestStep::Invalid)?;
            project
                .put_manifest_document(&document, true)
                .map_err(ManifestStep::Write)?;
            Ok(value)
        })
    }

    /// Read the current project again into the snapshot.
    fn reload_current(&mut self) -> Result<(), ReloadError> {
        let Some(project) = &self.current_project else {
            return Ok(());
        };
        let definition = project.get_project()?;
        self.current_definition = definition;
        let Some(workspace) = self.workspace.as_mut() else {
            return Ok(());
        };
        let Some(index) = workspace.member_index_of(project.root_path())? else {
            return Ok(());
        };
        let Some(ProjectDefinition::Toml(resolved)) = &self.current_definition else {
            return Err(ReloadError::NotAModelToml(project.root_path().to_owned()));
        };
        workspace
            .replace_member(index, (**resolved).clone())
            .map_err(|error| ReloadError::Workspace(Box::new(error)))
    }
}

#[cfg(test)]
#[path = "./context_tests.rs"]
mod tests;
