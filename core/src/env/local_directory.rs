// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use sha2::Sha256;
use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
};

use crate::{
    env::{PutProjectError, ReadEnvironment, WriteEnvironment, segment_uri_generic},
    project::local_src::LocalSrcProject,
};

use tempfile::{NamedTempFile, TempDir};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct LocalDirectoryEnvironment {
    pub environment_path: PathBuf,
}

pub const DEFAULT_ENV_NAME: &str = "sysand_env";

pub const ENTRIES_PATH: &str = "entries.txt";
pub const VERSIONS_PATH: &str = "versions.txt";

pub fn path_encode_uri<S: AsRef<str>>(uri: S) -> PathBuf {
    let mut result = PathBuf::new();
    for segment in segment_uri_generic::<S, Sha256>(uri) {
        result = result.join(segment);
    }

    result
}

pub fn remove_dir_if_empty<P: AsRef<Path>>(path: P) -> Result<(), std::io::Error> {
    match std::fs::remove_dir(path) {
        Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        r => r,
    }
}

pub fn remove_empty_dirs<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    let mut dirs: Vec<_> = walkdir::WalkDir::new(&path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();

    dirs.sort_by(|a, b| b.cmp(a));

    for dir in dirs {
        remove_dir_if_empty(&dir)?;
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum TryMoveError {
    #[error("failed but recovered")]
    RecoveredIOError(std::io::Error),
    #[error("failed and may have left directory in inconsistent state")]
    CatastrophicIOError {
        err: std::io::Error,
        cause: std::io::Error,
    },
}

fn try_remove_files<P: AsRef<Path>, I: Iterator<Item = P>>(paths: I) -> Result<(), TryMoveError> {
    let tempdir = tempfile::TempDir::new().map_err(TryMoveError::RecoveredIOError)?;
    let mut moved: Vec<PathBuf> = vec![];

    for (i, path) in paths.enumerate() {
        match std::fs::rename(&path, tempdir.path().join(i.to_string())) {
            Ok(_) => {
                moved.push(path.as_ref().to_path_buf());
            }
            Err(cause) => {
                // NOTE: This dance is to bypass the fact that std::io::error is not Clone-eable...
                let mut catastrophic_error = None;
                for (j, recover) in moved.iter().enumerate() {
                    if let Err(err) = std::fs::rename(tempdir.path().join(j.to_string()), recover) {
                        catastrophic_error = Some(err);
                        break;
                    }
                }

                if let Some(err) = catastrophic_error {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                } else {
                    return Err(TryMoveError::RecoveredIOError(cause));
                }
            }
        }
    }

    Ok(())
}

fn try_move_files(paths: &Vec<(&Path, &Path)>) -> Result<(), TryMoveError> {
    let tempdir = tempfile::TempDir::new().map_err(TryMoveError::RecoveredIOError)?;

    let mut last_err = None;

    // move source files out of the way
    for (i, (path, _)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{}", i));
        if let Err(e) = std::fs::rename(path, src_path) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = std::fs::rename(&src_path, path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIOError(cause));
    }

    let mut last_err = None;

    // Move target files out of the way
    for (i, (_, path)) in paths.iter().enumerate() {
        if path.exists() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));
            if let Err(e) = std::fs::rename(path, trg_path) {
                last_err = Some(e);
                break;
            }
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));

            if trg_path.exists() {
                if let Err(err) = std::fs::rename(&trg_path, path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = std::fs::rename(&src_path, path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIOError(cause));
    }

    let mut last_err = None;

    // Try moving files to destination
    for (i, (_, target)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{}", i));

        if let Err(e) = std::fs::rename(src_path, target) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (_, path)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if path.exists() {
                if let Err(err) = std::fs::rename(path, &src_path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));

            if trg_path.exists() {
                if let Err(err) = std::fs::rename(&trg_path, path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = std::fs::rename(&src_path, path) {
                    return Err(TryMoveError::CatastrophicIOError { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIOError(cause));
    }

    Ok(())
}

impl LocalDirectoryEnvironment {
    pub fn root_path(&self) -> PathBuf {
        self.environment_path.clone()
    }

    pub fn entries_path(&self) -> PathBuf {
        self.environment_path.join(ENTRIES_PATH)
    }

    pub fn uri_path<S: AsRef<str>>(&self, uri: S) -> PathBuf {
        self.environment_path.join(path_encode_uri(uri))
    }

    pub fn versions_path<S: AsRef<str>>(&self, uri: S) -> PathBuf {
        self.uri_path(uri).join(VERSIONS_PATH)
    }

    pub fn project_path<S: AsRef<str>, T: AsRef<str>>(&self, uri: S, version: T) -> PathBuf {
        self.uri_path(uri)
            .join(format!("{}.kpar", version.as_ref()))
    }
}

#[derive(Error, Debug)]
pub enum LocalReadError {
    //#[error("failed to read interchange project")]
    //FileReadError(#[from] crate::io::local_file::LocalReadError),
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
    // #[error("semver parse error")]
    // VersionParseError(#[from] semver::Error),
    // #[error("uri parse error")]
    // UriParseError(#[from] fluent_uri::error::ParseError<String>),
}

impl ReadEnvironment for LocalDirectoryEnvironment {
    type ReadError = LocalReadError;

    type UriIter = std::iter::Map<
        std::io::Lines<std::io::BufReader<std::fs::File>>,
        fn(Result<String, std::io::Error>) -> Result<String, LocalReadError>,
    >;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        Ok(
            std::io::BufReader::new(std::fs::File::open(self.entries_path())?)
                .lines()
                .map(|x| match x {
                    Ok(line) => Ok(line),
                    Err(err) => Err(LocalReadError::IOError(err)),
                }),
        )
    }

    type VersionIter = std::iter::Map<
        std::io::Lines<std::io::BufReader<std::fs::File>>,
        fn(Result<String, std::io::Error>) -> Result<String, LocalReadError>,
    >;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let vp = self.versions_path(uri);

        // TODO: Better refactor the interface to return a
        // maybe (similar to *Map::get)
        if !vp.exists() {
            if let Some(vpp) = vp.parent() {
                if !vpp.exists() {
                    std::fs::create_dir(vpp)?;
                }
            }
            std::fs::File::create(&vp)?;
        }

        Ok(std::io::BufReader::new(std::fs::File::open(vp)?)
            .lines()
            .map(|x| match x {
                Ok(line) => Ok(line),
                Err(err) => Err(LocalReadError::IOError(err)),
            }))
    }

    type InterchangeProjectRead = LocalSrcProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let project_path = self.project_path(uri, version).canonicalize()?;

        Ok(LocalSrcProject { project_path })
    }
}

#[derive(Error, Debug)]
pub enum LocalWriteError {
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("project deserialisation error")]
    SerialisationError(#[from] serde_json::Error),
    #[error("path error")]
    PathError(#[from] crate::project::local_src::PathError),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    //#[error("semver parse error")]
    // VersionParseError(#[from] semver::Error),
    // #[error("uri parse error")]
    // UriParseError(#[from] fluent_uri::error::ParseError<String>),
    // NOTE: Just treat as a general IOError
    //#[error{"project not found"}]
    //ProjectNotFound,
}

impl From<LocalReadError> for LocalWriteError {
    fn from(value: LocalReadError) -> Self {
        match value {
            LocalReadError::IOError(error) => LocalWriteError::IOError(error),
            //LocalReadError::VersionParseError(error) => LocalWriteError::VersionParseError(error),
            //LocalReadError::UriParseError(parse_error) => LocalWriteError::UriParseError(parse_error),
        }
    }
}

impl From<crate::project::local_src::LocalSrcError> for LocalWriteError {
    fn from(value: crate::project::local_src::LocalSrcError) -> Self {
        match value {
            crate::project::local_src::LocalSrcError::Serde(error) => {
                LocalWriteError::SerialisationError(error)
            }
            crate::project::local_src::LocalSrcError::Io(error) => LocalWriteError::IOError(error),
            crate::project::local_src::LocalSrcError::Path(path_error) => {
                LocalWriteError::PathError(path_error)
            }
            crate::project::local_src::LocalSrcError::AlreadyExists(msg) => {
                LocalWriteError::AlreadyExists(msg)
            }
        }
    }
}

fn add_line_temp<R: Read, S: AsRef<str>>(
    reader: R,
    line: S,
) -> Result<NamedTempFile, std::io::Error> {
    let mut temp_file = NamedTempFile::new()?;

    let mut line_added = false;
    for this_line in std::io::BufReader::new(reader).lines() {
        let this_line = this_line?;

        if !line_added && line.as_ref() < this_line.as_str() {
            writeln!(temp_file, "{}", line.as_ref())?;
            line_added = true;
        }

        writeln!(temp_file, "{}", this_line)?;

        if line.as_ref() == this_line {
            line_added = true;
        }
    }

    if !line_added {
        writeln!(temp_file, "{}", line.as_ref())?;
    }

    Ok(temp_file)
}

fn singleton_line_temp<S: AsRef<str>>(line: S) -> Result<NamedTempFile, std::io::Error> {
    let mut temp_file = NamedTempFile::new()?;

    writeln!(temp_file, "{}", line.as_ref())?;

    Ok(temp_file)
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
            std::fs::File::open(self.entries_path())
                .map_err(|e| PutProjectError::WriteError(LocalWriteError::IOError(e)))?,
            &uri,
        )
        .map_err(|e| PutProjectError::WriteError(LocalWriteError::IOError(e)))?;

        let versions_temp = if !versions_path.exists() {
            singleton_line_temp(version.as_ref())
        } else {
            let current_versions_f = std::fs::File::open(&versions_path)
                .map_err(|e| PutProjectError::WriteError(LocalWriteError::IOError(e)))?;
            add_line_temp(current_versions_f, version.as_ref())
        }
        .map_err(|e| PutProjectError::WriteError(LocalWriteError::IOError(e)))?;

        let project_temp =
            TempDir::new().map_err(|e| PutProjectError::WriteError(LocalWriteError::IOError(e)))?;

        let mut tentative_project = LocalSrcProject {
            project_path: project_temp.path().to_path_buf(),
        };

        write_project(&mut tentative_project).map_err(PutProjectError::CallbackError)?;

        // Project write was successful

        if !uri_path.exists() {
            std::fs::create_dir(&uri_path).map_err(|x| PutProjectError::WriteError(x.into()))?;
        }

        // Move existing stuff out of the way
        let project_path = self.project_path(&uri, &version);

        // TODO: Handle catastrophic errors differently
        try_move_files(&vec![
            (project_temp.path(), &project_path),
            (versions_temp.path(), &versions_path),
            (entries_temp.path(), &self.entries_path()),
        ])
        .map_err(|e| match e {
            TryMoveError::RecoveredIOError(err) => {
                PutProjectError::WriteError(LocalWriteError::IOError(err))
            }
            TryMoveError::CatastrophicIOError { err, cause: _ } => {
                PutProjectError::WriteError(LocalWriteError::IOError(err))
            }
        })?;

        Ok(LocalSrcProject { project_path })
    }

    fn del_project_version<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        uri: S,
        version: T,
    ) -> Result<(), Self::WriteError> {
        let mut versions_temp = NamedTempFile::with_suffix("versions.txt")?;

        let versions_path = self.versions_path(&uri);
        let mut found = false;
        let mut empty = true;

        // I think this may be needed on Windows in order to drop the
        // file handle before overwriting
        {
            let current_versions_f = std::io::BufReader::new(std::fs::File::open(&versions_path)?);
            for version_line_ in current_versions_f.lines() {
                let version_line = version_line_?;

                if version.as_ref() != version_line {
                    writeln!(versions_temp, "{}", version_line)?;
                    empty = false;
                } else {
                    found = true;
                }
            }
        }

        if found {
            let project = self.get_project(&uri, version)?;

            // TODO: Add better error messages for catastrophic errors
            if let Err(err) = try_remove_files(project.get_source_paths()?.into_iter().chain(vec![
                project.root_path().join(".project.json"),
                project.root_path().join(".meta.json"),
            ])) {
                match err {
                    TryMoveError::RecoveredIOError(error) => {
                        return Err(LocalWriteError::IOError(error));
                    }
                    TryMoveError::CatastrophicIOError {
                        err: error,
                        cause: _,
                    } => {
                        // Censor the version if a partial delete happened, better pretend
                        // like it does not exist than to pretend like a broken
                        // package is properly installed
                        std::fs::copy(versions_temp.path(), &versions_path)?;
                        return Err(LocalWriteError::IOError(error));
                    }
                }
            }

            std::fs::copy(versions_temp.path(), &versions_path)?;

            remove_empty_dirs(project.root_path())?;

            if empty {
                let current_uris_: Result<Vec<String>, LocalReadError> = self.uris()?.collect();
                let current_uris: Vec<String> = current_uris_?;
                let mut f = std::io::BufWriter::new(std::fs::File::create(self.entries_path())?);
                for existing_uri in current_uris {
                    if uri.as_ref() != existing_uri {
                        writeln!(f, "{}", existing_uri)?;
                    }
                }
                std::fs::remove_file(self.versions_path(&uri))?;
                remove_dir_if_empty(self.uri_path(uri))?;
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
