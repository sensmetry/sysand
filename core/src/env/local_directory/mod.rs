// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::{self, BufRead, BufReader, Write};

use camino::Utf8PathBuf;
use camino_tempfile::NamedUtf8TempFile;
use thiserror::Error;

use crate::{
    env::{PutProjectError, ReadEnvironment, WriteEnvironment},
    project::{
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{
            FsIoError, ProjectDeserializationError, ProjectSerializationError, ToPathBuf, wrapfs,
        },
    },
};

pub mod manifest;
mod utils;

use utils::{
    TryMoveError, add_line_temp, path_encode_uri, remove_dir_if_empty, remove_empty_dirs,
    singleton_line_temp, try_move_files, try_remove_files,
};

#[derive(Clone, Debug)]
pub struct LocalDirectoryEnvironment {
    pub environment_path: Utf8PathBuf,
}

pub const DEFAULT_ENV_NAME: &str = "sysand_env";

pub const DEFAULT_MANIFEST_NAME: &str = "current.toml";

pub const ENTRIES_PATH: &str = "entries.txt";
pub const VERSIONS_PATH: &str = "versions.txt";

impl LocalDirectoryEnvironment {
    pub fn root_path(&self) -> Utf8PathBuf {
        self.environment_path.clone()
    }

    pub fn entries_path(&self) -> Utf8PathBuf {
        self.environment_path.join(ENTRIES_PATH)
    }

    pub fn uri_path<S: AsRef<str>>(&self, uri: S) -> Utf8PathBuf {
        self.environment_path.join(path_encode_uri(uri))
    }

    pub fn versions_path<S: AsRef<str>>(&self, uri: S) -> Utf8PathBuf {
        let mut p = self.uri_path(uri);
        p.push(VERSIONS_PATH);
        p
    }

    pub fn project_path<S: AsRef<str>, T: AsRef<str>>(&self, uri: S, version: T) -> Utf8PathBuf {
        let mut p = self.uri_path(uri);
        p.push(format!("{}.kpar", version.as_ref()));
        p
    }
}

#[derive(Error, Debug)]
pub enum LocalReadError {
    #[error("failed to read project list file `entries.txt`: {0}")]
    ProjectListFileRead(io::Error),
    #[error("failed to read project versions file `versions.txt`: {0}")]
    ProjectVersionsFileRead(io::Error),
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

    type UriIter = std::iter::Map<
        io::Lines<BufReader<std::fs::File>>,
        fn(Result<String, io::Error>) -> Result<String, LocalReadError>,
    >;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        Ok(BufReader::new(wrapfs::File::open(self.entries_path())?)
            .lines()
            .map(|x| match x {
                Ok(line) => Ok(line),
                Err(err) => Err(LocalReadError::ProjectListFileRead(err)),
            }))
    }

    type VersionIter = std::iter::Map<
        io::Lines<BufReader<std::fs::File>>,
        fn(Result<String, io::Error>) -> Result<String, LocalReadError>,
    >;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let vp = self.versions_path(uri);

        // TODO: Better refactor the interface to return a
        // maybe (similar to *Map::get)
        if !vp.exists() {
            if let Some(vpp) = vp.parent() {
                if !vpp.exists() {
                    wrapfs::create_dir(vpp)?;
                }
            }
            wrapfs::File::create(&vp)?;
        }

        Ok(BufReader::new(wrapfs::File::open(&vp)?)
            .lines()
            .map(|x| match x {
                Ok(line) => Ok(line),
                Err(err) => Err(LocalReadError::ProjectVersionsFileRead(err)),
            }))
    }

    type InterchangeProjectRead = LocalSrcProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let path = self.project_path(uri, version);
        let project_path = wrapfs::canonicalize(path)?;

        Ok(LocalSrcProject { project_path })
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
            e @ (LocalReadError::ProjectListFileRead(_)
            | LocalReadError::ProjectVersionsFileRead(_)) => Self::LocalRead(e),
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
        }
    }
}

impl WriteEnvironment for LocalDirectoryEnvironment {
    type WriteError = LocalWriteError;

    type InterchangeProjectMut = LocalSrcProject;

    fn put_project<S: AsRef<str>, T: AsRef<str>, F, E>(
        &mut self,
        uri: S,
        version: T,
        write_project: F,
    ) -> Result<Self::InterchangeProjectMut, PutProjectError<Self::WriteError, E>>
    where
        F: FnOnce(&mut Self::InterchangeProjectMut) -> Result<(), E>,
    {
        let uri_path = self.uri_path(&uri);
        let versions_path = self.versions_path(&uri);

        let entries_temp = add_line_temp(
            wrapfs::File::open(self.entries_path()).map_err(LocalWriteError::from)?,
            &uri,
        )?;

        let versions_temp = if !versions_path.exists() {
            singleton_line_temp(version.as_ref())
        } else {
            let current_versions_f =
                wrapfs::File::open(&versions_path).map_err(LocalWriteError::from)?;
            add_line_temp(current_versions_f, version.as_ref())
        }?;

        let project_temp = camino_tempfile::tempdir()
            .map_err(|e| LocalWriteError::from(FsIoError::MkTempDir(e)))?;

        let mut tentative_project = LocalSrcProject {
            project_path: project_temp.path().to_path_buf(),
        };

        write_project(&mut tentative_project).map_err(PutProjectError::Callback)?;

        // Project write was successful

        if !uri_path.exists() {
            wrapfs::create_dir(&uri_path).map_err(LocalWriteError::from)?;
        }

        // Move existing stuff out of the way
        let project_path = self.project_path(&uri, &version);

        // TODO: Handle catastrophic errors differently
        try_move_files(&vec![
            (project_temp.path(), &project_path),
            (versions_temp.path(), &versions_path),
            (entries_temp.path(), &self.entries_path()),
        ])
        .map_err(LocalWriteError::from)?;

        Ok(LocalSrcProject { project_path })
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        let mut versions_temp =
            NamedUtf8TempFile::with_suffix("versions.txt").map_err(FsIoError::CreateTempFile)?;

        let versions_path = self.versions_path(&uri);
        let mut found = false;
        let mut empty = true;

        // I think this may be needed on Windows in order to drop the
        // file handle before overwriting
        {
            let current_versions_f = BufReader::new(wrapfs::File::open(&versions_path)?);
            for version_line_ in current_versions_f.lines() {
                let version_line = version_line_
                    .map_err(|e| FsIoError::ReadFile(versions_path.to_path_buf(), e))?;

                if version.as_ref() != version_line {
                    writeln!(versions_temp, "{}", version_line)
                        .map_err(|e| FsIoError::WriteFile(versions_path.clone(), e))?;

                    empty = false;
                } else {
                    found = true;
                }
            }
        }

        if found {
            let project: LocalSrcProject = self
                .get_project(&uri, version)
                .map_err(LocalWriteError::from)?;

            // TODO: Add better error messages for catastrophic errors
            if let Err(err) = try_remove_files(project.get_source_paths()?.into_iter().chain(vec![
                project.project_path.join(".project.json"),
                project.project_path.join(".meta.json"),
            ])) {
                match err {
                    TryMoveError::CatastrophicIO { .. } => {
                        // Censor the version if a partial delete happened, better pretend
                        // like it does not exist than to pretend like a broken
                        // package is properly installed
                        wrapfs::copy(versions_temp.path(), &versions_path)?;
                        return Err(err.into());
                    }
                    TryMoveError::RecoveredIO(_) => return Err(LocalWriteError::from(err)),
                }
            }

            wrapfs::copy(versions_temp.path(), &versions_path)?;

            remove_empty_dirs(project.project_path)?;
            if empty {
                let current_uris_: Result<Vec<String>, LocalReadError> = self.uris()?.collect();
                let current_uris: Vec<String> = current_uris_?;
                let entries_path = self.entries_path();
                let mut f = io::BufWriter::new(wrapfs::File::create(&entries_path)?);
                for existing_uri in current_uris {
                    if uri.as_ref() != existing_uri {
                        writeln!(f, "{}", existing_uri)
                            .map_err(|e| FsIoError::WriteFile(entries_path.clone(), e))?;
                    }
                }
                wrapfs::remove_file(versions_path)?;
                remove_dir_if_empty(self.uri_path(&uri))?;
            }
        }

        Ok(())
    }

    fn del_uri<S: AsRef<str>>(&mut self, uri: S) -> Result<(), Self::WriteError> {
        let current_uris_: Result<Vec<String>, LocalReadError> = self.uris()?.collect();
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
