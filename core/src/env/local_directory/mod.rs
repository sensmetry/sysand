// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    fs,
    io::{ErrorKind, Write},
    vec,
};

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use thiserror::Error;

use crate::{
    env::{
        PutProjectError, ReadEnvironment, WriteEnvironment,
        iri_normalize::IriVersionFilename,
        local_directory::{
            metadata::{
                AddProjectError, EnvMetadata, EnvMetadataError, EnvProject, load_env_metadata,
                parse_env_metadata,
            },
            utils::clean_dir,
        },
    },
    lock::{Lock, Source},
    project::{
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{
            FsIoError, ProjectDeserializationError, ProjectSerializationError, RelativizePathError,
            wrapfs,
        },
    },
    workspace::Workspace,
};

pub mod metadata;
pub mod utils;

use utils::{TryMoveError, try_move_files};

// TODO: avoid cloning, maybe use `Arc`?
/// `sysand_env` metadata. Metadata changes have to be written to `env.toml` explicitly
#[derive(Debug, Clone)]
pub struct LocalDirectoryEnvironment {
    /// Path of the env, including `sysand_env` part. Must be canonical
    root_dir: Utf8PathBuf,
    metadata: EnvMetadata,
}

pub const METADATA_PATH: &str = "env.toml";
pub const PROJECT_PATH_PREFIX: &str = "lib/";

impl LocalDirectoryEnvironment {
    /// `root_dir` can be any cwd-relative/absolute path
    pub fn read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, EnvMetadataError> {
        let root_dir = wrapfs::canonicalize(root_dir)?;
        let metadata = load_env_metadata(root_dir.join(METADATA_PATH))?;
        Ok(Self { root_dir, metadata })
    }

    /// `root_dir` can be any cwd-relative/absolute path. `env.toml` must not exist
    pub fn create<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, Box<FsIoError>> {
        let root_dir = wrapfs::canonicalize(root_dir)?;

        let metadata = EnvMetadata::default();
        let path = root_dir.join(METADATA_PATH);
        let mut file = wrapfs::File::create_new(&path)?;
        file.write_all(metadata.to_string().as_bytes())
            .map_err(|e| FsIoError::WriteFile(path, e))?;

        Ok(Self { root_dir, metadata })
    }

    /// Try reading the environment metadata. If it does not exist,
    /// returns `Ok(None)`.
    /// `root_dir` can be any cwd-relative/absolute path
    pub fn try_read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Option<Self>, EnvMetadataError> {
        let root_dir = root_dir.as_ref();
        let meta_path = root_dir.join(METADATA_PATH);
        match fs::read_to_string(&meta_path) {
            Ok(s) => {
                let metadata = parse_env_metadata(meta_path, s)?;

                let root_dir = wrapfs::canonicalize(root_dir)?;
                Ok(Some(Self { root_dir, metadata }))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(EnvMetadataError::Io(
                FsIoError::ReadFile(meta_path, e).into(),
            )),
        }
    }

    // TODO: Integrate the updating of editable metadata into `WriteEnvironment` trait.
    //       This will likely require updating it to support
    //       multiple identifiers per project.
    /// Precondition: sync has completed, i.e. projects from `lock` are installed
    /// by `self.put_project()`.
    /// Call is idempotent.
    /// Does not update metadata file.
    pub fn merge_lock(&mut self, lock: &Lock, ws: Option<&Workspace>) {
        for project in &lock.projects {
            // Projects that are installed in the environment are ignored, so only
            // editable (and workspace, which are a subset of editable) projects have to be added
            if let [Source::Editable { editable }, ..] = project.sources.as_slice() {
                let usages = project
                    .usages
                    .iter()
                    .map(|usage| usage.resource.clone())
                    .collect();

                let workspace_member = ws
                    .map(|w| w.projects().iter().any(|p| p.path.as_str() == editable))
                    .unwrap_or_default();
                // This is called once per `sync`, so has to be idempotent
                if let Some(existing) = self
                    .metadata
                    .find_project_version_any_mut(&project.identifiers, &project.version)
                {
                    assert_eq!(existing.workspace, workspace_member);
                    assert!(existing.editable);

                    for iri in &project.identifiers {
                        if !existing.identifiers.contains(iri) {
                            existing.identifiers.push(iri.to_owned());
                        }
                    }
                    existing.path = editable.as_str().into();
                    existing.usages = usages;
                } else {
                    self.metadata.projects.push(EnvProject {
                        publisher: project.publisher.to_owned(),
                        name: project.name.to_owned(),
                        version: project.version.to_owned(),
                        path: editable.as_str().into(),
                        identifiers: project.identifiers.to_owned(),
                        usages,
                        editable: true,
                        workspace: workspace_member,
                    });
                }
            }
        }
    }

    pub fn root_path(&self) -> &Utf8Path {
        &self.root_dir
    }

    pub fn metadata_path(&self) -> Utf8PathBuf {
        self.root_dir.join(METADATA_PATH)
    }

    pub fn write(&self) -> Result<(), Box<FsIoError>> {
        wrapfs::write(self.metadata_path(), self.metadata.to_string())
    }

    pub fn projects(&self) -> &[EnvProject] {
        &self.metadata.projects
    }

    /// Determine absolute path of `project`
    fn absolute_project_path(&self, project: &EnvProject) -> Utf8PathBuf {
        if project.editable {
            self.parent_dir().join(project.path.as_str())
        } else {
            self.root_dir.join(project.path.as_str())
        }
    }

    // /// Find a project `uri` version `version` and determine its absolute path
    // pub fn absolute_project_path_find<S: AsRef<str>, T: AsRef<str>>(
    //     &self,
    //     uri: S,
    //     version: T,
    // ) -> Option<Utf8PathBuf> {
    //     self.metadata
    //         .find_project_version(uri, version)
    //         .map(|p| self.project_to_absolute_path(p))
    // }

    // /// Project path relative to the env directory
    // pub fn relative_project_path_find<S: AsRef<str>, T: AsRef<str>>(
    //     &self,
    //     uri: S,
    //     version: T,
    // ) -> Option<Utf8PathBuf> {
    //     self.metadata.find_project_version(uri, version).map(|p| {
    //         if p.editable {
    //             // TODO: this assumes that parent is workspace root
    //             Utf8Path::new("../").join(p.path.as_str())
    //         } else {
    //             p.path.as_str().into()
    //         }
    //     })
    // }

    /// Parent directory of the env, i.e. the directory in which `sysand_env` resides.
    /// It is assumed to be the workspace (if present) or project root, which in turn is
    /// the root of relative paths of `editable`/`workspace` projects
    // TODO: is it correct to assume that `sysand_env` is always at workspace/project root?
    fn parent_dir(&self) -> &Utf8Path {
        // Will fail only if env is at root, i.e. `self.root_dir == /`
        self.root_dir.parent().unwrap()
    }

    // fn project_to_absolute_path(&self, project: &EnvProject) -> Utf8PathBuf {
    //     if project.editable {
    //         self.parent_dir().join(project.path.as_str())
    //     } else {
    //         self.root_dir.join(project.path.as_str())
    //     }
    // }

    fn ensure_lib_dir_exists(&self) -> Result<(), Box<FsIoError>> {
        let lib_dir = self.root_dir.join(PROJECT_PATH_PREFIX);
        match fs::create_dir(&lib_dir) {
            Ok(()) => Ok(()),
            Err(e) => {
                if e.kind() != ErrorKind::AlreadyExists {
                    Err(FsIoError::MkDir(lib_dir, e).into())
                } else {
                    Ok(())
                }
            }
        }
    }

    fn get_project_storage(&self, project: &EnvProject) -> LocalSrcProject {
        let relative = project.path.as_str();
        if project.editable {
            // let absolute = self.parent_dir().join(relative);
            // We will assume that the relative path was constructed by us
            // by diffing canonical paths of the project and env parent (i.e. workspace root)
            // Otherwise canonicalization would be needed here to remove possible
            // (internal) `../` of editable projects, which can reside anywhere.
            // In principle thus it should be enough to strip however many `../` are
            // at the start of `relative` from `parent`, and then join (stripped) relative
            // to (stripped) parent.
            let mut absolute = self.parent_dir().to_path_buf();
            for c in Utf8Path::new(relative).components() {
                match c {
                    Utf8Component::ParentDir => {
                        absolute.pop();
                    }
                    Utf8Component::Normal(c) => absolute.push(c),
                    Utf8Component::CurDir | Utf8Component::Prefix(_) | Utf8Component::RootDir => {
                        unreachable!()
                    }
                }
            }
            LocalSrcProject {
                nominal_path: Some(relative.into()),
                project_path: absolute,
            }
        } else {
            let absolute = self.root_dir.join(relative);
            let relative = format!("{}/{relative}", self.root_dir.file_name().unwrap());
            LocalSrcProject {
                nominal_path: Some(relative.into()),
                project_path: absolute,
            }
        }
    }

    /// Determine a path for a new project/version. Path will be relative to `self.root_path()`
    fn compute_project_path(&self, iri: Iri<&str>, version: impl AsRef<str>) -> Utf8PathBuf {
        let mut path_iter = IriVersionFilename::new(iri, version);
        let mut candidate = path_iter.next_candidate();
        while self.metadata.project_dir_exists(candidate) {
            candidate = path_iter.next_candidate();
        }
        let mut path = String::from(path_iter);
        path.insert_str(0, PROJECT_PATH_PREFIX);
        path.into()
    }
}

#[derive(Error, Debug)]
pub enum LocalReadError {
    #[error("prioject {0} is not present in environment")]
    ProjectNotFound(Box<str>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for LocalReadError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl ReadEnvironment for LocalDirectoryEnvironment {
    type ReadError = LocalReadError;

    // TODO: avoid this mess of wrapping in result, avoid
    // collecting
    type UriIter = Vec<Result<String, LocalReadError>>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        Ok(self
            .metadata
            .projects
            .iter()
            .flat_map(|p| p.identifiers.iter())
            .cloned()
            .map(Ok)
            .collect())
    }

    type VersionIter = Vec<Result<String, LocalReadError>>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let identifier = uri.as_ref();
        Ok(self
            .metadata
            .find_project(identifier)
            .map(|p| Ok(p.version.to_owned()))
            .collect())
    }

    type InterchangeProjectRead = LocalSrcProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        if let Some(project) = self.metadata.find_project_version(&uri, &version) {
            Ok(self.get_project_storage(project))
        } else {
            Err(LocalReadError::ProjectNotFound(uri.as_ref().into()))
        }
    }
}

#[derive(Error, Debug)]
pub enum LocalWriteError {
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error("path error: {0}")]
    Path(#[from] PathError),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    TryMove(#[from] TryMoveError),
    #[error(transparent)]
    LocalRead(LocalReadError),
    #[error(transparent)]
    AddProject(#[from] AddProjectError),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
}

impl From<FsIoError> for LocalWriteError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalReadError> for LocalWriteError {
    fn from(value: LocalReadError) -> Self {
        match value {
            LocalReadError::Io(error) => Self::Io(error),
            LocalReadError::ProjectNotFound(_) => Self::LocalRead(value),
        }
    }
}

impl From<LocalSrcError> for LocalWriteError {
    fn from(value: LocalSrcError) -> Self {
        match value {
            LocalSrcError::Deserialize(error) => LocalWriteError::Deserialize(error),
            LocalSrcError::Path(path_error) => LocalWriteError::Path(path_error),
            LocalSrcError::AlreadyExists(msg) => LocalWriteError::AlreadyExists(msg),
            LocalSrcError::Io(e) => LocalWriteError::Io(e),
            LocalSrcError::Serialize(error) => Self::Serialize(error),
            LocalSrcError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalSrcError::MissingMeta => LocalWriteError::MissingMeta,
        }
    }
}

impl WriteEnvironment for LocalDirectoryEnvironment {
    type WriteError = LocalWriteError;

    type InterchangeProjectMut = LocalSrcProject;

    /// Will overwrite the specified project version if it exists
    fn put_project<S: AsRef<str>, T: AsRef<str>, F, CE>(
        &mut self,
        uri: S,
        version: T,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, CE>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), CE>,
    {
        let identifier = uri.as_ref();
        let version = version.as_ref();

        let project_temp = camino_tempfile::tempdir()
            .map_err(|e| LocalWriteError::from(FsIoError::MkTempDir(e)))?;
        let mut tentative_project = LocalSrcProject {
            nominal_path: None,
            project_path: project_temp.path().to_path_buf(),
        };

        if let Some(existing) = self.metadata.find_project_version(identifier, version) {
            // Create a temp clone and change it to avoid modifying env in case of errors
            // TODO: check that publisher/name match?
            // Will assume that usages/publisher/name/etc remain unchanged
            // TODO: how to handle editable projects here?
            assert!(!existing.editable);
            assert!(!existing.workspace);

            write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;

            let path = self.absolute_project_path(existing);
            try_move_files(&[(project_temp.path(), &path)]).map_err(LocalWriteError::from)?;

            // Metadata didn't change, nothing to write

            tentative_project.project_path = self.root_path().join(&path);
            tentative_project.nominal_path = Some(path);

            Ok(tentative_project)
        } else {
            // TODO: be optimistic: move existing target out of the way, try writing to
            // the target directly and on failure revert.
            write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;

            // Project write was successful

            // TODO: take iri as arg
            let iri = Iri::parse(identifier)
                .map_err(|e| PutProjectError::IriParse(identifier.to_owned(), e))?;
            let path = self.compute_project_path(iri, version);
            let absolute_project_path = self.root_path().join(&path);

            // Tolerate missing `lib/`
            self.ensure_lib_dir_exists()
                .map_err(LocalWriteError::from)?;

            // Move existing stuff out of the way
            // TODO: Handle catastrophic errors differently
            try_move_files(&[(project_temp.path(), &absolute_project_path)])
                .map_err(LocalWriteError::from)?;

            tentative_project.project_path = absolute_project_path;
            tentative_project.nominal_path = Some(path);

            self.metadata
                .add_local_project(
                    vec![identifier.to_owned()],
                    &tentative_project,
                    false,
                    false,
                )
                .map_err(LocalWriteError::from)?;

            self.write().map_err(LocalWriteError::from)?;

            Ok(tentative_project)
        }
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        let identifier = uri.as_ref();
        let version = version.as_ref();
        if let Some((idx, project)) = self.metadata.find_project_version_idx(identifier, version) {
            // Doesn't make sense to remove workspace projects
            assert!(!project.workspace);
            // Editable projects are not owned by the env
            if !project.editable {
                // TODO: maybe surface IO errors?
                let project_dir = self.root_dir.join(project.path.as_str());
                clean_dir(&project_dir);
                let _ = fs::remove_dir(&project_dir)
                    .map_err(|e| log::warn!("failed to remove empty dir `{project_dir}`: {e}"));
            }
            self.metadata.projects.swap_remove(idx);
            self.write()?;
        }

        Ok(())
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        let project_versions = self.metadata.find_project_idx(uri.as_ref());
        let mut indices_to_remove = Vec::new();
        for (idx, p) in project_versions {
            // Doesn't make sense to remove workspace projects
            assert!(!p.workspace);
            if !p.editable {
                // TODO: maybe surface IO errors?
                let project_dir = self.root_dir.join(p.path.as_str());
                clean_dir(&project_dir);
                let _ = fs::remove_dir(&project_dir)
                    .map_err(|e| log::warn!("failed to remove empty dir `{project_dir}`: {e}"));
            }
            indices_to_remove.push(idx);
        }
        // `swap_remove()` does not affect elements before the one being removed,
        // so indices have to be removed from largest to smallest
        indices_to_remove.sort_unstable();
        for idx in indices_to_remove.iter().copied().rev() {
            self.metadata.projects.swap_remove(idx);
        }
        self.write()?;

        Ok(())
    }
}
