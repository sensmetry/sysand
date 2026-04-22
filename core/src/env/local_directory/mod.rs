// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, io::ErrorKind, slice, vec};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use thiserror::Error;

use crate::{
    env::{
        PutProjectError, ReadEnvironment, WriteEnvironment,
        local_directory::metadata::{
            AddProjectError, EnvMetadata, EnvMetadataError, EnvProject, load_env_metadata,
            parse_env_metadata,
        },
        utils::IriVersionFilename,
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
mod utils;

use utils::{TryMoveError, remove_empty_dirs, try_move_files, try_remove_files};

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
    /// `root_dir` must be canonical
    pub fn read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, EnvMetadataError> {
        // TODO: make sure all callers use canonical paths
        // debug_assert_eq!(root_dir, wrapfs::canonicalize(&root_dir).unwrap());

        let root_dir = wrapfs::canonicalize(root_dir).unwrap();
        let metadata = load_env_metadata(root_dir.join(METADATA_PATH))?;
        Ok(Self { root_dir, metadata })
    }

    /// `root_dir` must be canonical
    pub fn create<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, Box<FsIoError>> {
        // TODO: make sure all callers use canonical paths
        // debug_assert_eq!(root_dir, wrapfs::canonicalize(&root_dir).unwrap());

        let root_dir = wrapfs::canonicalize(root_dir).unwrap();

        let metadata = EnvMetadata::default();
        wrapfs::write(root_dir.join(METADATA_PATH), metadata.to_string())?;

        Ok(Self { root_dir, metadata })
    }

    /// Read the environment metadata or return empty metadata. No
    /// filesystem writes are performed.
    /// If the metadata path exists, it must be a valid file
    /// `root_dir` must be canonical
    pub fn read_or_default<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, EnvMetadataError> {
        // TODO: make sure all callers use canonical paths
        // debug_assert_eq!(root_dir, wrapfs::canonicalize(&root_dir).unwrap());

        let root_dir = wrapfs::canonicalize(root_dir).unwrap();

        let meta_path = root_dir.join(METADATA_PATH);
        match fs::read_to_string(&meta_path) {
            Ok(s) => {
                let metadata = parse_env_metadata(meta_path, s)?;
                Ok(Self { root_dir, metadata })
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Self {
                root_dir,
                metadata: EnvMetadata::default(),
            }),
            Err(e) => Err(EnvMetadataError::Io(
                FsIoError::ReadFile(meta_path, e).into(),
            )),
        }
    }

    /// Try reading the environment metadata. If it does not exist,
    /// returns `Ok(None)`
    pub fn try_read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Option<Self>, EnvMetadataError> {
        let root_dir = root_dir.as_ref();
        let meta_path = root_dir.join(METADATA_PATH);
        match fs::read_to_string(&meta_path) {
            Ok(s) => {
                let metadata = parse_env_metadata(meta_path, s)?;

                let root_dir = wrapfs::canonicalize(root_dir).unwrap();
                Ok(Some(Self { root_dir, metadata }))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(EnvMetadataError::Io(
                FsIoError::ReadFile(meta_path, e).into(),
            )),
        }
    }

    // TODO:
    // - if there is identifier overlap and name/version match between projects in lock and env,
    //   merge identifiers
    // TODO: Integrate the updating of editable metadata into `WriteEnvironment` trait.
    //       This will likely require updating it to support
    //       multiple identifiers per project.
    /// Precondition: sync has completed, i.e. projects from `lock` are installed by `put_project`
    /// in `self`.
    /// Call is not idempotent.
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

    pub fn root_path(&self) -> &Utf8Path {
        &self.root_dir
    }

    pub fn metadata_path(&self) -> Utf8PathBuf {
        self.root_dir.join(METADATA_PATH)
    }

    pub fn write(&self) -> Result<(), Box<FsIoError>> {
        wrapfs::write(self.metadata_path(), self.metadata.to_string())
    }

    /// Determine absolute path of `project`
    pub fn absolute_project_path(&self, project: &EnvProject) -> Utf8PathBuf {
        if project.editable {
            // Will assume that workspace root is the parent of `sysand_env`
            // FIXME: be more robust
            self.root_dir.parent().unwrap().join(project.path.as_str())
        } else {
            self.root_dir.join(project.path.as_str())
        }
    }

    pub fn projects(&self) -> &[EnvProject] {
        &self.metadata.projects
    }

    /// Find a project `uri` version `version` and determine its absolute path
    pub fn absolute_project_path_find<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Option<Utf8PathBuf> {
        self.metadata
            .find_project_version(slice::from_ref(&uri), version)
            .map(|p| {
                if p.editable {
                    // TODO: this assumes that parent is workspace root
                    self.root_dir.parent().unwrap().join(p.path.as_str())
                } else {
                    self.root_dir.join(p.path.as_str())
                }
            })

        // let mut p = self.uri_path(uri);
        // p.push(format!("{}.kpar", version.as_ref()));
        // p
    }

    /// Project path relative to the env directory
    pub fn relative_project_path<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Option<Utf8PathBuf> {
        self.metadata
            .find_project_version(slice::from_ref(&uri), version)
            .map(|p| {
                if p.editable {
                    // TODO: this assumes that parent is workspace root
                    Utf8Path::new("../").join(p.path.as_str())
                } else {
                    p.path.as_str().into()
                }
            })

        // let mut p = self.uri_path(uri);
        // p.push(format!("{}.kpar", version.as_ref()));
        // p
    }

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

    /// Determine a path for a new project/version. Path will be relative to `self.root_path()`
    fn compute_project_path(&self, iri: impl AsRef<str>, version: impl AsRef<str>) -> Utf8PathBuf {
        let iri = Iri::parse(iri.as_ref()).unwrap();
        let mut path_iter = IriVersionFilename::new(&iri, version);
        let mut candidate = path_iter.next_candidate();
        while self.metadata.project_dir_exists(candidate) {
            candidate = path_iter.next_candidate();
        }
        let mut path = String::from(candidate);
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

    // type UriIter = std::iter::Map<
    //     io::Lines<BufReader<std::fs::File>>,
    //     fn(Result<String, io::Error>) -> Result<String, LocalReadError>,
    // >;
    // TODO: avoid this mess of wrapping in result
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
        // Ok(BufReader::new(wrapfs::File::open(self.entries_path())?)
        //     .lines()
        //     .map(|x| match x {
        //         Ok(line) => Ok(line),
        //         Err(err) => Err(LocalReadError::ProjectListFileRead(err)),
        //     }))
    }

    // TODO: be more efficient, do not create a Vec
    type VersionIter = Vec<Result<String, LocalReadError>>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        // let vp = self.versions_path(uri);
        let identifier = uri.as_ref();
        Ok(self
            .metadata
            .find_project(slice::from_ref(&identifier))
            .map(|p| Ok(p.version.to_owned()))
            .collect())
        // // TODO: Better refactor the interface to return a
        // // maybe (similar to *Map::get)
        // if !vp.exists() {
        //     if let Some(vpp) = vp.parent()
        //         && !vpp.exists()
        //     {
        //         wrapfs::create_dir(vpp)?;
        //     }
        //     wrapfs::File::create(&vp)?;
        // }

        // Ok(BufReader::new(wrapfs::File::open(&vp)?)
        //     .lines()
        //     .map(|x| match x {
        //         Ok(line) => Ok(line),
        //         Err(err) => Err(LocalReadError::ProjectVersionsFileRead(err)),
        //     }))
    }

    type InterchangeProjectRead = LocalSrcProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let Some(absolute_path) = self.absolute_project_path_find(&uri, version) else {
            return Err(LocalReadError::ProjectNotFound(uri.as_ref().into()));
        };
        // Canonicalization is needed here to remove possible `../` of
        // editable projects, which can reside anywhere
        let absolute_path = wrapfs::canonicalize(absolute_path)?;
        let env_parent = self
            .root_path()
            // Will fail only if env is at root
            .parent()
            .unwrap();
        let nominal_path = absolute_path
            .strip_prefix(env_parent)
            // Will not fail unless absolute_project_path is buggy
            // TODO: deal with windows different drive paths (in lockfile,env,new dep types) like
            // Cargo does: relative paths where possible, absolute paths for different drives and such
            .unwrap()
            .to_path_buf();

        Ok(LocalSrcProject {
            nominal_path: Some(nominal_path),
            project_path: absolute_path,
        })
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

        if let Some(existing_idx) = self
            .metadata
            .find_project_version_idx(slice::from_ref(&identifier), version)
        {
            // Create a temp clone and change it to avoid modifying env in case of errors
            // TODO: check that publisher/name match?
            // Will assume that usages/publisher/name/etc remain unchanged
            let existing = &self.metadata.projects[existing_idx];
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

            let path = self.compute_project_path(identifier, version);
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
        let existing = self
            .metadata
            .find_project_version_idx(slice::from_ref(&identifier), version);

        if let Some(idx) = existing {
            // TODO: return Option<something> from get_project()
            let project: LocalSrcProject = self
                .get_project(identifier, version)
                .map_err(LocalWriteError::from)?;

            // TODO: Add better error messages for catastrophic errors
            if let Err(err) = try_remove_files(
                project
                    .get_source_paths()?
                    .into_iter()
                    .chain(vec![project.info_path(), project.meta_path()]),
            ) {
                match err {
                    TryMoveError::CatastrophicIO { .. } => {
                        // Still remove version from metadata if a partial delete happened,
                        // better pretend like it does not exist than to pretend like a broken
                        // package is properly installed
                        self.metadata.projects.remove(idx);
                        self.write().map_err(LocalWriteError::from)?;
                        return Err(err.into());
                    }
                    // Failed to remove project
                    TryMoveError::RecoveredIO(_) => return Err(LocalWriteError::from(err)),
                }
            }

            self.metadata.projects.remove(idx);
            self.write().map_err(LocalWriteError::from)?;

            remove_empty_dirs(project.project_path)?;
        }
        Ok(())
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        let current_uris_: Result<Vec<String>, LocalReadError> = self.uris()?.into_iter().collect();
        let current_uris: Vec<String> = current_uris_?;

        if current_uris.contains(&uri.as_ref().to_string()) {
            for version_ in self.versions(&uri)? {
                let version: String = version_?;
                self.del_project_version(&uri, &version)?;
            }
        }

        Ok(())
    }
}
