// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    debug_assert_matches, fs,
    io::{ErrorKind, Write as _},
    vec,
};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;

use crate::{
    env::{
        ProjectChecksum, ProjectChecksumResult, PutProjectError, ReadEnvironment, WriteEnvironment,
        local_directory::{
            metadata::{
                AddProjectError, EnvMetadata, EnvMetadataError, EnvProject, EnvProjectChecksum,
                load_env_metadata, parse_env_metadata,
            },
            utils::clean_dir,
        },
    },
    iri_normalize::IriVersionFilename,
    lock::{Lock, Source},
    project::{
        ProjectRead as _,
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{
            FsIoError, Identifier, ProjectDeserializationError, ProjectSerializationError,
            RelativizePathError, wrapfs,
        },
    },
    utils::join_lexical,
    workspace_lock::ExclusiveLock,
};

pub mod metadata;
pub mod utils;

use utils::{TryMoveError, try_move_files};

// TODO: validate IRIs and paths
// TODO: avoid cloning this struct, maybe use `Arc`?
/// `.sysand` metadata. Metadata changes have to be written to `env.toml` explicitly
#[derive(Debug, Clone)]
pub struct LocalDirectoryEnvironment {
    /// Path of the env, including `.sysand` part. Must be canonical
    root_dir: Utf8PathBuf,
    metadata: EnvMetadata,
    /// The advisory lock the writer holds, handed in by [`Self::hold`];
    /// `None` for a reader, or for an embedder that did not take one.
    lock: Option<ExclusiveLock>,
}

pub const METADATA_PATH: &str = "env.toml";
pub const PROJECT_PATH_PREFIX: &str = "lib/";

impl LocalDirectoryEnvironment {
    /// `root_dir` can be any cwd-relative/absolute path
    pub fn read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, EnvMetadataError> {
        let root_dir = wrapfs::canonicalize(root_dir)?;

        Self::warn_if_old_sysand_env_present(&root_dir);

        let metadata = load_env_metadata(root_dir.join(METADATA_PATH))?;
        Ok(Self {
            root_dir,
            metadata,
            lock: None,
        })
    }

    /// `root_dir` can be any cwd-relative/absolute path. `env.toml` must not exist
    pub fn create<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Self, Box<FsIoError>> {
        let root_dir = wrapfs::canonicalize(root_dir)?;

        Self::warn_if_old_sysand_env_present(&root_dir);

        let metadata = EnvMetadata::default();
        let path = root_dir.join(METADATA_PATH);
        let mut file = wrapfs::File::create_new(&path)?;
        file.write_all(metadata.to_string().as_bytes())
            .map_err(|e| FsIoError::WriteFile(path, e))?;

        Ok(Self {
            root_dir,
            metadata,
            lock: None,
        })
    }

    /// Try reading the environment metadata. If it does not exist,
    /// returns `Ok(None)`.
    /// `root_dir` can be any cwd-relative/absolute path
    pub fn try_read<P: AsRef<Utf8Path>>(root_dir: P) -> Result<Option<Self>, EnvMetadataError> {
        let root_dir = root_dir.as_ref();

        Self::warn_if_old_sysand_env_present(root_dir);

        let meta_path = root_dir.join(METADATA_PATH);
        match fs::read_to_string(&meta_path) {
            Ok(s) => {
                let metadata = parse_env_metadata(meta_path, s)?;

                let root_dir = wrapfs::canonicalize(root_dir)?;
                Ok(Some(Self {
                    root_dir,
                    metadata,
                    lock: None,
                }))
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
    /// Updates list of projects present in env metadata, but not installed in the
    /// environment (for now this includes only `editable` projects). All previous
    /// non-installed projects are removed first.
    /// To install a project in the environment, use `put_project()`.
    /// Call is idempotent. Does not update metadata file
    ///
    /// An editable project at one of `workspace_members` (see
    /// [`crate::workspace::model_toml::workspace_members`]) is marked as a
    /// workspace project.
    pub fn merge_lock(&mut self, lock: &Lock, workspace_members: &[Utf8UnixPathBuf]) {
        self.metadata.projects.retain(Self::holds_copy);
        for project in &lock.projects {
            // Projects that are installed in the environment are ignored, so only
            // editable (and workspace, which are a subset of editable) projects have to be added
            if let Some(Source::Editable { editable }) = project.sources.first() {
                let usages = project
                    .usages
                    .iter()
                    .map(|usage| usage.to_string())
                    .collect();

                self.metadata.projects.push(EnvProject {
                    publisher: project.publisher.clone(),
                    name: project.name.clone(),
                    version: project.version.clone(),
                    path: editable.as_str().into(),
                    identifiers: project
                        .identifiers
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                    usages,
                    editable: true,
                    workspace: workspace_members.contains(editable),
                    checksum: None,
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

    pub fn projects(&self) -> &[EnvProject] {
        &self.metadata.projects
    }

    /// Hand the environment the workspace lock its writer holds, so its
    /// writes run under it.
    ///
    /// Fails when the lock is on a different root from the one this
    /// environment is in.
    pub fn hold(&mut self, lock: ExclusiveLock) -> Result<(), ForeignLockError> {
        if lock.root() != self.parent_dir() {
            return Err(ForeignLockError {
                lock_root: lock.root().to_owned(),
                env: self.root_dir.clone(),
            });
        }
        self.lock = Some(lock);
        Ok(())
    }

    /// Note a write that runs without an exclusive workspace lock; an
    /// embedder may serialize writers its own way.
    fn note_unlocked_write(&self, what: &str) {
        if self.lock.is_none() {
            log::debug!(
                "{what} on `{}` without an exclusive workspace lock",
                self.root_dir
            );
        }
    }

    /// Directories directly under `lib/` that no recorded project owns,
    /// relative to the environment root, sorted.
    /// Editable and workspace entries live outside `lib/` and are not
    /// consulted.
    pub fn unrecorded_paths(&self) -> Result<Vec<Utf8UnixPathBuf>, Box<FsIoError>> {
        let lib_dir = self.root_dir.join(PROJECT_PATH_PREFIX);
        if !wrapfs::is_dir(&lib_dir)? {
            return Ok(vec![]);
        }
        let mut unrecorded = Vec::new();
        for entry in wrapfs::read_dir(&lib_dir)? {
            let entry = entry.map_err(|e| Box::new(FsIoError::ReadDir(lib_dir.clone(), e)))?;
            let relative =
                Utf8UnixPathBuf::from(format!("{PROJECT_PATH_PREFIX}{}", entry.file_name()));
            if !self
                .metadata
                .projects
                .iter()
                .any(|project| project.path == relative)
            {
                unrecorded.push(relative);
            }
        }
        unrecorded.sort();
        Ok(unrecorded)
    }

    /// Parent directory of the env, i.e. the directory in which `.sysand` resides.
    /// It is assumed to be the workspace (if present) or project root, which in turn is
    /// the root of relative paths of `editable`/`workspace` projects
    // TODO: is it correct to assume that `.sysand` is always at workspace/project root?
    fn parent_dir(&self) -> &Utf8Path {
        // Will fail only if env is at root, i.e. `self.root_dir == /`
        self.root_dir.parent().unwrap()
    }

    fn ensure_lib_dir_exists(&self) -> Result<(), Box<FsIoError>> {
        let lib_dir = self.root_dir.join(PROJECT_PATH_PREFIX);
        match fs::create_dir(&lib_dir) {
            Ok(()) => Ok(()),
            Err(e) => {
                if e.kind() == ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(FsIoError::MkDir(lib_dir, e).into())
                }
            }
        }
    }

    // TODO: an environment holds every installed version of a project, and
    // `EnvResolver` hands all of them to the solver, so resolving through one
    // is a version choice and ought to default a version constraint the way
    // an index does. A plain `LocalSrcProject` answers
    // `source_may_offer_multiple_versions()` with `false`, because a path
    // names one project, so a `pkg:sysand` usage met only by the environment
    // admits a prerelease where the index would not. The environment should
    // say so when it constructs the project.
    fn get_project_storage(&self, project: &EnvProject) -> LocalSrcProject {
        let relative = project.path.as_str();
        if project.editable {
            // Assume that the relative path was constructed by us
            // by diffing canonical paths of the project and env parent (i.e. workspace root)
            // Otherwise canonicalization would be needed here to remove possible
            // (internal) `../` of editable projects, which can reside anywhere.
            // In principle thus it should be enough to strip however many `../` are
            // at the start of `relative` from `parent`, and then join (stripped) relative
            // to (stripped) parent.
            let absolute = join_lexical(self.parent_dir(), Utf8Path::new(relative));
            // Editable projects must not have checksums
            debug_assert_matches!(project.checksum, None);
            LocalSrcProject::new_access(absolute, Some(relative.into()))
        } else {
            let absolute = self.root_dir.join(relative);
            let relative = format!("{}/{relative}", self.root_dir.file_name().unwrap()).into();
            match project.src_checksum() {
                Some(checksum) => LocalSrcProject::new_access_with_checksum(
                    absolute,
                    Some(relative),
                    checksum.to_owned(),
                ),
                None => LocalSrcProject::new_access(absolute, Some(relative)),
            }
        }
    }

    /// Whether `p` is installed in the environment, i.e. lives under `lib/`.
    /// The refusal to remove `project`, if it is not a copy the env holds.
    fn not_installed(identifier: &str, project: &EnvProject) -> Option<LocalWriteError> {
        (project.editable || project.workspace).then(|| LocalWriteError::RemoveNotInstalled {
            identifier: identifier.into(),
            version: project.version.as_str().into(),
            kind: if project.workspace {
                "a workspace"
            } else {
                "an editable"
            },
        })
    }

    fn holds_copy(p: &EnvProject) -> bool {
        p.path.starts_with(PROJECT_PATH_PREFIX)
    }

    /// Determine a path for a new project/version. Path will be relative to and under
    /// `self.root_path()`
    fn compute_project_path(&self, iri: Iri<&str>, version: impl AsRef<str>) -> Utf8UnixPathBuf {
        let mut path_iter = IriVersionFilename::new(iri, version);
        let mut candidate = path_iter.next_candidate();
        while self
            .metadata
            .projects
            .iter()
            .any(|p| Self::holds_copy(p) && p.path.file_name().unwrap() == candidate)
        {
            candidate = path_iter.next_candidate();
        }
        let mut path = String::from(path_iter);
        path.insert_str(0, PROJECT_PATH_PREFIX);
        path.into()
    }

    fn warn_if_old_sysand_env_present(root_dir: &Utf8Path) {
        let parent = root_dir.parent().unwrap();
        let path = parent.join("sysand_env");
        match fs::metadata(&path) {
            Ok(m) => {
                if m.is_dir() {
                    log::warn!(
                        "`sysand_env` directory was found; it is no longer used by sysand and should be removed"
                    );
                }
            }
            Err(e) => {
                if e.kind() != ErrorKind::NotFound {
                    log::debug!("failed to get metadata of old env at `{path}`: {e}")
                }
            }
        }
    }
}

/// A lock handed to [`LocalDirectoryEnvironment::hold`] guards another
/// directory, so writes under it would not be serialized.
#[derive(Error, Debug)]
#[error("the workspace lock on `{lock_root}` does not guard the environment at `{env}`")]
pub struct ForeignLockError {
    pub lock_root: Utf8PathBuf,
    pub env: Utf8PathBuf,
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
        // `projects` has one entry per installed version, in the order they
        // were installed; list each project's first identifier once.
        let mut uris: Vec<&str> = Vec::new();
        for project in &self.metadata.projects {
            if let Some(uri) = project.identifiers.first()
                && !uris.contains(&uri.as_str())
            {
                uris.push(uri);
            }
        }
        Ok(uris.into_iter().map(|uri| Ok(uri.to_owned())).collect())
    }

    type VersionIter = Vec<Result<String, LocalReadError>>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let identifier = uri.as_ref();
        Ok(self
            .metadata
            .find_project_versions(identifier)
            .map(|p| Ok(p.version.clone()))
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

    fn has_version_verified<S: AsRef<str>, V: AsRef<str>>(
        &self,
        uri: S,
        version: V,
        checksum: &ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError> {
        if let Some(p) = self.metadata.find_project_version(uri, version) {
            let (expected, actual) = match (checksum, &p.checksum) {
                (_, None) => return Ok(ProjectChecksumResult::ChecksumNotPresent),
                (ProjectChecksum::Project(c), Some(EnvProjectChecksum::Project { src_cksum })) => {
                    (c, src_cksum)
                }
                (ProjectChecksum::Kpar(c), Some(EnvProjectChecksum::Kpar { kpar_cksum })) => {
                    (c, kpar_cksum)
                }
                _ => return Ok(ProjectChecksumResult::DifferentChecksumKinds),
            };
            if expected == actual {
                Ok(ProjectChecksumResult::Match)
            } else {
                Ok(ProjectChecksumResult::Mismatch)
            }
        } else {
            Ok(ProjectChecksumResult::VersionNotFound)
        }
    }
}

#[derive(Error, Debug)]
pub enum LocalWriteError {
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error("path error")]
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
        directory to one of its dependencies' directory"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error(
        "cannot install `{identifier}` {version}: the environment already has this version\n\
        as {kind} project, which cannot be overwritten"
    )]
    OverwriteNotInstalled {
        identifier: Box<str>,
        version: Box<str>,
        /// "an editable" or "a workspace"
        kind: &'static str,
    },
    #[error("project is missing metadata file `.meta.json`")]
    MissingMeta,
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    #[error("project `{0}` not found in env")]
    ProjectNotFound(String),
    /// The entry points at a project used in place; the lockfile, not
    /// the environment, decides whether it is there.
    #[error(
        "project `{identifier}` {version} is {kind} project, which is not installed in the env\n\
        {SP:>8} and cannot be removed from it",
        SP = ' '
    )]
    RemoveNotInstalled {
        identifier: Box<str>,
        version: Box<str>,
        /// "an editable" or "a workspace"
        kind: &'static str,
    },
    /// A directory whose manifests do not say what kind of project it
    /// is, or that is a `model.toml` project and so cannot be written
    /// as a json one.
    #[error(transparent)]
    Format(LocalSrcError),
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
            LocalSrcError::Deserialize(error) => Self::Deserialize(error),
            LocalSrcError::Path(path_error) => Self::Path(path_error),
            LocalSrcError::AlreadyExists(msg) => Self::AlreadyExists(msg),
            LocalSrcError::Io(e) => Self::Io(e),
            LocalSrcError::Serialize(error) => Self::Serialize(error),
            LocalSrcError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalSrcError::MissingMeta => Self::MissingMeta,
            error @ LocalSrcError::MissingInfoMeta { missing: _ } => Self::Format(error),
            error @ (LocalSrcError::AmbiguousFormat { dir: _, json: _ }
            | LocalSrcError::TomlProjectHasNoChecksum { dir: _ }
            | LocalSrcError::JsonWriteToTomlProject { dir: _, file: _ }
            | LocalSrcError::TomlWriteToJsonProject { dir: _ }) => Self::Format(error),
        }
    }
}

impl WriteEnvironment for LocalDirectoryEnvironment {
    type WriteError = LocalWriteError;

    type InterchangeProjectMut = LocalSrcProject;

    /// Will overwrite the specified project version if it exists
    // TODO: support multiple identifiers. This will allow aliases and identifying
    // projects by their publisher/name, even if they were retrieved under a
    // different identifier.
    fn put_project<S: AsRef<str>, T: AsRef<str>, F, CE>(
        &mut self,
        uri: S,
        version: T,
        checksum: Option<ProjectChecksum>,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, CE>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), CE>,
    {
        let identifier = uri.as_ref();
        let version = version.as_ref();
        self.note_unlocked_write("put_project");

        let project_temp = camino_tempfile::tempdir()
            .map_err(|e| LocalWriteError::from(FsIoError::MkTempDir(e)))?;
        let mut tentative_project =
            LocalSrcProject::new_access(project_temp.path().to_path_buf(), None);

        if let Some(existing) = self.metadata.find_project_version_mut(identifier, version) {
            // Editable and workspace projects are not managed by the env
            if existing.editable || existing.workspace {
                return Err(PutProjectError::Write(
                    LocalWriteError::OverwriteNotInstalled {
                        identifier: identifier.into(),
                        version: version.into(),
                        kind: if existing.workspace {
                            "a workspace"
                        } else {
                            "an editable"
                        },
                    },
                ));
            }

            // Write into a temp dir first so a failure leaves the env untouched
            write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;
            // Project is not editable, so this is always correct
            let absolute_path = self.root_dir.join(existing.path.as_str());
            try_move_files(&[(project_temp.path(), &absolute_path)])
                .map_err(LocalWriteError::from)?;
            tentative_project.set_path(absolute_path);

            let definition = match tentative_project.get_project() {
                Ok(Some(definition)) => definition,
                Ok(None) => return Err(PutProjectError::Write(LocalWriteError::MissingInfoMeta)),
                Err(e) => return Err(PutProjectError::Write(LocalWriteError::from(e))),
            };

            existing.publisher = definition.publisher().map(ToOwned::to_owned);
            existing.name = definition.name().to_owned();
            existing.version = definition.version();
            existing.usages = definition
                .usages()
                .into_iter()
                .map(|u| Identifier::from_interchange_usage_unchecked(&u).into_string())
                .collect();
            existing.checksum = checksum.map(Into::into);
        } else {
            // TODO: try writing to the target directly (we manage it exclusively) and on failure revert.
            write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;

            // TODO: take iri as arg
            let iri = Iri::parse(identifier)
                .map_err(|e| PutProjectError::IriParse(identifier.to_owned(), e))?;
            let path = self.compute_project_path(iri, version);
            let absolute_project_path = self.root_path().join(path.as_str());

            // Tolerate missing `lib/`
            self.ensure_lib_dir_exists()
                .map_err(LocalWriteError::from)?;

            // TODO: Handle catastrophic errors differently
            try_move_files(&[(project_temp.path(), &absolute_project_path)])
                .map_err(LocalWriteError::from)?;

            tentative_project.set_path(absolute_project_path);
            tentative_project.set_nominal_path(Some(path));

            self.metadata
                .add_local_project(
                    vec![identifier.to_owned()],
                    &tentative_project,
                    false,
                    false,
                    checksum,
                )
                .map_err(LocalWriteError::from)?;
        }
        self.write().map_err(LocalWriteError::from)?;
        Ok(tentative_project)
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        let identifier = uri.as_ref();
        let version = version.as_ref();
        self.note_unlocked_write("del_project_version");
        if let Some((idx, project)) = self.metadata.find_project_version_idx(identifier, version) {
            if let Some(error) = Self::not_installed(identifier, project) {
                return Err(error);
            }
            // TODO: maybe surface IO errors?
            let project_dir = self.root_dir.join(project.path.as_str());
            log::debug!(
                "removing project `{identifier}` ({version}) from env at `{}`",
                self.root_dir
            );
            clean_dir(&project_dir);
            if let Err(e) = fs::remove_dir(&project_dir) {
                log::warn!("failed to remove empty dir `{project_dir}`: {e}");
            }
            self.metadata.projects.swap_remove(idx);
            self.write()?;
        } else {
            return Err(LocalWriteError::ProjectNotFound(identifier.to_owned()));
        }

        Ok(())
    }

    fn is_installed<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<bool, Self::WriteError> {
        Ok(self
            .metadata
            .find_project_version_idx(uri.as_ref(), version.as_ref())
            .is_some_and(|(_, project)| Self::holds_copy(project)))
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        self.note_unlocked_write("del_uri");
        let mut project_versions = self
            .metadata
            .find_project_versions_idxs(uri.as_ref())
            .peekable();
        if project_versions.peek().is_none() {
            return Err(LocalWriteError::ProjectNotFound(uri.as_ref().to_owned()));
        }
        let project_versions: Vec<_> = project_versions.collect();
        for (_, p) in &project_versions {
            if let Some(error) = Self::not_installed(uri.as_ref(), p) {
                return Err(error);
            }
        }
        let mut indices_to_remove = Vec::new();
        for (idx, p) in project_versions {
            // TODO: maybe surface IO errors?
            let project_dir = self.root_dir.join(p.path.as_str());
            log::debug!(
                "removing project `{}` ({}) from env at `{}`",
                uri.as_ref(),
                p.version,
                self.root_dir
            );
            clean_dir(&project_dir);
            if let Err(e) = fs::remove_dir(&project_dir) {
                log::warn!("failed to remove empty dir `{project_dir}`: {e}");
            }
            indices_to_remove.push(idx);
        }
        // `swap_remove()` does not affect elements before the one being removed,
        // so indices have to be removed from largest to smallest
        for idx in indices_to_remove.iter().copied().rev() {
            self.metadata.projects.swap_remove(idx);
        }
        self.write()?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
