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
    manifest::{
        MODEL_TOML_FILE,
        edit::{self, DocumentMut},
        kpar_view,
        project::ResolvedProject,
    },
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    paths::{RelativizePathError, relativize_path},
    project::{
        ManifestError, ManifestWriteError, ProjectDefinition, ProjectFormat, ProjectRead as _,
        edit_manifest_document,
        local_src::{LocalSrcError, LocalSrcProject, detect_format, validated_json_documents},
        utils::{FsIoError, ProjectDeserializationError, wrapfs},
    },
    workspace::model_toml::{DiscoverError, LoadError, ReadMemberError, Workspace},
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

/// The current project's manifest as it was, to put back with
/// [`ProjectContext::restore_manifest`].
#[cfg(feature = "filesystem")]
#[derive(Debug, Clone)]
pub struct ManifestBackup(BackedUp);

#[cfg(feature = "filesystem")]
#[derive(Debug, Clone)]
enum BackedUp {
    Toml(String),
    Json {
        info: Box<InterchangeProjectInfoRaw>,
        meta: Box<InterchangeProjectMetadataRaw>,
    },
}

/// Why the manifest could not be backed up or put back.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("there is no current project")]
    NoProject,
    #[error("the current project has no manifest")]
    NoManifest,
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Write(#[from] LocalSrcError),
    #[error(transparent)]
    Reload(#[from] ReloadError),
    /// The backup is of a manifest in the other format.
    #[error("the manifest is not in the format the backup was taken in")]
    FormatChanged,
}

/// Why a member could not be added to the workspace.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum AddMemberError<E> {
    #[error("there is no workspace to add a member to")]
    NoWorkspace,
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    /// The workspace's `model.toml` is not one that can be edited.
    #[error(transparent)]
    Manifest(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Edit(#[from] edit::EditError),
    /// The workspace would not load with the member.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Writing the member failed.
    #[error(transparent)]
    Member(E),
}

/// A change to the current project's manifest, in the form each of the two
/// manifest formats takes it.
///
/// The edit runs on the manifest as of the context's snapshot, and
/// [`ProjectContext::edit`] writes the result only when it validates.
#[cfg(feature = "filesystem")]
pub trait ManifestEdit {
    type Output;
    type Error;

    /// Edit the `model.toml` `document`, whose resolved manifest is `project`.
    fn toml(
        self,
        document: &mut DocumentMut,
        project: &ResolvedProject,
    ) -> Result<Self::Output, Self::Error>;

    /// Edit the json documents of `project`.
    fn json(
        self,
        project: &LocalSrcProject,
        info: &mut InterchangeProjectInfoRaw,
        meta: &mut InterchangeProjectMetadataRaw,
    ) -> Result<Self::Output, Self::Error>;
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
    /// The member's `model.toml` could not be read again.
    #[error(transparent)]
    Member(#[from] ReadMemberError),
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
    #[error("the current project has no manifest")]
    NoManifest,
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
    Read(#[from] ManifestError<LocalSrcError>),
    #[error(transparent)]
    Edit(E),
    /// The edited manifest does not resolve, so it is not written.
    #[error(transparent)]
    Invalid(ProjectDeserializationError),
    /// The workspace would not hold together with the edited manifest, so
    /// it is not written.
    #[error(transparent)]
    Workspace(Box<LoadError>),
    #[error(transparent)]
    Write(#[from] ManifestWriteError<LocalSrcError>),
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

    /// [`crate::workspace::project_at_path`], where a member of the loaded
    /// workspace is that member as loaded. Any other path is placed by a
    /// walk, as it may belong to another workspace.
    pub fn project_at_path(
        &self,
        path: impl Into<Utf8PathBuf>,
        nominal_path: Option<Utf8UnixPathBuf>,
    ) -> Result<LocalSrcProject, DiscoverError> {
        let path = path.into();
        if let Some(workspace) = &self.workspace
            && let Some(member) = workspace.member_at(&path)?
        {
            return Ok(LocalSrcProject::new_member(
                path,
                nominal_path,
                member.project.clone(),
            ));
        }
        crate::workspace::project_at_path(path, nominal_path)
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
                .config()
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
        let index = self
            .current_member_index()
            .map_err(|error| EditError::Reload(ReloadError::Io(error)))?;
        let Some(project) = self.current_project.as_mut() else {
            return Err(EditError::NoProject);
        };
        let edited = edit(project);
        let reloaded = self.reload_current(index);
        match edited {
            Err(error) => Err(EditError::Edit(error)),
            Ok(value) => match reloaded {
                Ok(()) => Ok(value),
                Err(error) => Err(EditError::Reload(error)),
            },
        }
    }

    /// [`ProjectContext::edit_project`] for a `model.toml` project:
    /// loads the manifest document, applies `edit` to it, and writes it,
    /// refusing a manifest that does not resolve. A member's manifest is
    /// resolved by its workspace.
    ///
    /// The snapshot becomes the resolution of the written text. Only a
    /// failed write can have changed the file, so only then is the
    /// snapshot read again. In a workspace the checks that span members
    /// run before the write.
    pub fn edit_manifest<T, E>(
        &mut self,
        edit: impl FnOnce(&mut DocumentMut) -> Result<T, E>,
    ) -> Result<T, EditError<ManifestStep<E>>> {
        let index = self
            .current_member_index()
            .map_err(|error| EditError::Reload(ReloadError::Io(error)))?;
        let Some(project) = self.current_project.as_mut() else {
            return Err(EditError::NoProject);
        };
        let member = self.workspace.as_ref().zip(index);
        let edited = edit_manifest_document(project, |_, document| {
            let value = edit(document).map_err(ManifestStep::Edit)?;
            let text = document.to_string();
            let resolved = match member {
                Some((workspace, index)) => workspace.resolve_member(index, &text),
                None => kpar_view::resolved_of_text(MODEL_TOML_FILE, &text),
            }
            .map_err(ManifestStep::Invalid)?;
            let workspace = member
                .map(|(workspace, index)| workspace.with_member_replaced(index, resolved.clone()))
                .transpose()
                .map_err(|error| ManifestStep::Workspace(Box::new(error)))?;
            Ok((value, resolved, workspace))
        });
        match edited {
            Ok((value, resolved, workspace)) => {
                if let Some(project) = self.current_project.as_mut()
                    && workspace.is_some()
                {
                    project.set_member_manifest(resolved.clone());
                }
                if workspace.is_some() {
                    self.workspace = workspace;
                }
                self.current_definition = Some(ProjectDefinition::Toml(Box::new(resolved)));
                Ok(value)
            }
            Err(error @ ManifestStep::Write(_)) => {
                // The write's error is the cause, and the one returned.
                #[expect(
                    clippy::let_underscore_must_use,
                    let_underscore_drop,
                    reason = "the snapshot is brought up to date; the write's error is reported"
                )]
                // ast-grep-ignore: let-underscore-result
                let _ = self.reload_current(index);
                Err(EditError::Edit(error))
            }
            Err(error) => Err(EditError::Edit(error)),
        }
    }

    /// Apply `edit` to the current project's manifest, whichever format it
    /// has, and write the result when it validates as reading it would.
    ///
    /// The snapshot becomes the validated result. Only a failed write can
    /// have changed a file, so only then is the snapshot read again.
    pub fn edit<E: ManifestEdit>(
        &mut self,
        edit: E,
    ) -> Result<E::Output, EditError<ManifestStep<E::Error>>> {
        match &self.current_definition {
            Some(ProjectDefinition::Toml(resolved)) => {
                let resolved = (**resolved).clone();
                self.edit_manifest(|document| edit.toml(document, &resolved))
            }
            Some(ProjectDefinition::Json { info: _, meta: _ }) => {
                self.edit_json(|project, info, meta| edit.json(project, info, meta))
            }
            None if self.current_project.is_some() => Err(EditError::NoManifest),
            None => Err(EditError::NoProject),
        }
    }

    /// [`ProjectContext::edit`] for a json project.
    fn edit_json<T, E>(
        &mut self,
        edit: impl FnOnce(
            &LocalSrcProject,
            &mut InterchangeProjectInfoRaw,
            &mut InterchangeProjectMetadataRaw,
        ) -> Result<T, E>,
    ) -> Result<T, EditError<ManifestStep<E>>> {
        let Some(ProjectDefinition::Json {
            info: before_info,
            meta: before_meta,
        }) = &self.current_definition
        else {
            return Err(EditError::NoManifest);
        };
        let Some(project) = &self.current_project else {
            return Err(EditError::NoProject);
        };
        let mut info = before_info.clone();
        let mut meta = before_meta.clone();
        let value = edit(project, &mut info, &mut meta)
            .map_err(|error| EditError::Edit(ManifestStep::Edit(error)))?;
        let (info, meta) = validated_json_documents(&info, &meta)
            .map_err(|error| EditError::Edit(ManifestStep::Invalid(error)))?;
        let written = project.write_json_documents((before_info, before_meta), (&info, &meta));
        match written {
            Ok(()) => {
                self.current_definition = Some(ProjectDefinition::Json { info, meta });
                Ok(value)
            }
            Err(error) => {
                // A json project is never a workspace member.
                #[expect(
                    clippy::let_underscore_must_use,
                    let_underscore_drop,
                    reason = "the snapshot is brought up to date; the write's error is reported"
                )]
                // ast-grep-ignore: let-underscore-result
                let _ = self.reload_current(None);
                Err(EditError::Edit(ManifestStep::Write(
                    ManifestWriteError::Project(error),
                )))
            }
        }
    }

    /// Add a member to the workspace: `relative` is appended to the
    /// `members` of the root `model.toml`, and `member_text` is the
    /// member's `model.toml`, which `write_member` writes.
    ///
    /// The workspace must load with the member before anything is written.
    /// The member is written first, then the root, so a failed write leaves
    /// no `members` entry that names nothing.
    pub fn add_member<E>(
        &mut self,
        relative: &str,
        member_text: &str,
        write_member: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), AddMemberError<E>> {
        let Some(workspace) = &self.workspace else {
            return Err(AddMemberError::NoWorkspace);
        };
        let file = workspace.root().join(MODEL_TOML_FILE);
        let text = wrapfs::read_to_string(&file)?;
        let mut document = edit::validated(file.as_str(), &text)?;
        edit::push_member(&mut document, relative)?;
        let edited = edit::checked_text(file.as_str(), &document)?;
        let with_member = workspace.with_member_added(&edited, member_text)?;
        write_member().map_err(AddMemberError::Member)?;
        wrapfs::write_atomic(&file, edited)?;
        self.workspace = Some(with_member);
        Ok(())
    }

    /// The current project's manifest as it is, for
    /// [`ProjectContext::restore_manifest`]. A `model.toml` is read from
    /// disk, as the snapshot does not keep its text.
    pub fn backup_manifest(&self) -> Result<ManifestBackup, BackupError> {
        let Some(project) = &self.current_project else {
            return Err(BackupError::NoProject);
        };
        match &self.current_definition {
            Some(ProjectDefinition::Toml(_)) => Ok(ManifestBackup(BackedUp::Toml(
                wrapfs::read_to_string(project.root_path().join(MODEL_TOML_FILE))?,
            ))),
            Some(ProjectDefinition::Json { info, meta }) => Ok(ManifestBackup(BackedUp::Json {
                info: Box::new(info.clone()),
                meta: Box::new(meta.clone()),
            })),
            None => Err(BackupError::NoManifest),
        }
    }

    /// Write the manifest `backup` holds back, over whatever is there.
    pub fn restore_manifest(&mut self, backup: &ManifestBackup) -> Result<(), BackupError> {
        let index = self.current_member_index()?;
        let Some(project) = &self.current_project else {
            return Err(BackupError::NoProject);
        };
        match (&backup.0, &self.current_definition) {
            (BackedUp::Toml(text), Some(ProjectDefinition::Toml(_))) => {
                wrapfs::write_atomic(project.root_path().join(MODEL_TOML_FILE), text)?;
                Ok(self.reload_current(index)?)
            }
            (
                BackedUp::Json { info, meta },
                Some(ProjectDefinition::Json {
                    info: current_info,
                    meta: current_meta,
                }),
            ) => {
                project.write_json_documents((current_info, current_meta), (&**info, &**meta))?;
                self.current_definition = Some(ProjectDefinition::Json {
                    info: (**info).clone(),
                    meta: (**meta).clone(),
                });
                Ok(())
            }
            (_, Some(_)) => Err(BackupError::FormatChanged),
            (_, None) => Err(BackupError::NoManifest),
        }
    }

    /// The index of the workspace member the current project is, if it is
    /// one.
    fn current_member_index(&self) -> Result<Option<usize>, Box<FsIoError>> {
        match (&self.workspace, &self.current_project) {
            (Some(workspace), Some(project)) => workspace.member_index_of(project.root_path()),
            _ => Ok(None),
        }
    }

    /// Read the current project again into the snapshot. `index` is the
    /// member the current project is, if it is one; a member is read by its
    /// workspace.
    fn reload_current(&mut self, index: Option<usize>) -> Result<(), ReloadError> {
        let Some(project) = &self.current_project else {
            return Ok(());
        };
        let (Some(workspace), Some(index)) = (&self.workspace, index) else {
            self.current_definition = project.get_project()?;
            return Ok(());
        };
        if detect_format(project.root_path())? != ProjectFormat::Toml {
            return Err(ReloadError::NotAModelToml(project.root_path().to_owned()));
        }
        let resolved = workspace.reread_member(index)?;
        self.settle(Some(index), resolved)
    }

    /// Put `resolved`, the current project's manifest as it is on disk,
    /// into the snapshot. `index` is the member the current project is, if
    /// it is one; the workspace then runs the checks that span members.
    fn settle(
        &mut self,
        index: Option<usize>,
        resolved: ResolvedProject,
    ) -> Result<(), ReloadError> {
        let member = match (
            self.workspace.as_mut(),
            self.current_project.as_mut(),
            index,
        ) {
            (Some(workspace), Some(project), Some(index)) => {
                project.set_member_manifest(resolved.clone());
                Some((workspace, index))
            }
            _ => None,
        };
        let replaced = match member {
            Some((workspace, index)) => workspace.replace_member(index, resolved.clone()),
            None => Ok(()),
        };
        self.current_definition = Some(ProjectDefinition::Toml(Box::new(resolved)));
        replaced.map_err(|error| ReloadError::Workspace(Box::new(error)))
    }
}

#[cfg(test)]
#[path = "./context_tests.rs"]
mod tests;
