// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::NamedUtf8TempFile;
use sha2::Sha256;
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
};

use crate::{
    env::{PutProjectError, ReadEnvironment, WriteEnvironment, segment_uri_generic},
    project::{
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{
            FsIoError, ProjectDeserializationError, ProjectSerializationError, ToPathBuf, wrapfs,
        },
    },
};

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct LocalDirectoryEnvironment {
    pub environment_path: Utf8PathBuf,
}

pub const DEFAULT_ENV_NAME: &str = "sysand_env";

pub const ENTRIES_PATH: &str = "entries.txt";
pub const VERSIONS_PATH: &str = "versions.txt";

/// Get a relative path corresponding to the given `uri`
pub fn path_encode_uri<S: AsRef<str>>(uri: S) -> Utf8PathBuf {
    let mut result = Utf8PathBuf::new();
    for segment in segment_uri_generic::<S, Sha256>(uri) {
        result.push(segment);
    }

    result
}

pub fn remove_dir_if_empty<P: AsRef<Utf8Path>>(path: P) -> Result<(), FsIoError> {
    match fs::remove_dir(path.as_ref()) {
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
        r => r.map_err(|e| FsIoError::RmDir(path.to_path_buf(), e)),
    }
}

pub fn remove_empty_dirs<P: AsRef<Utf8Path>>(path: P) -> Result<(), FsIoError> {
    let mut dirs: Vec<_> = walkdir::WalkDir::new(path.as_ref())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_type()
                .is_dir()
                .then(|| Utf8PathBuf::from_path_buf(e.into_path()).ok())
                .flatten()
        })
        .collect();

    dirs.sort_by(|a, b| b.cmp(a));

    for dir in dirs {
        remove_dir_if_empty(&dir)?;
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum TryMoveError {
    #[error("recovered from failure: {0}")]
    RecoveredIO(Box<FsIoError>),
    #[error(
        "failed and may have left the directory in inconsistent state:\n{err}\nwhich was caused by:\n{cause}"
    )]
    CatastrophicIO {
        err: Box<FsIoError>,
        cause: Box<FsIoError>,
    },
}

fn try_remove_files<P: AsRef<Utf8Path>, I: Iterator<Item = P>>(
    paths: I,
) -> Result<(), TryMoveError> {
    let tempdir = camino_tempfile::tempdir()
        .map_err(|e| TryMoveError::RecoveredIO(FsIoError::CreateTempFile(e).into()))?;
    let mut moved: Vec<Utf8PathBuf> = vec![];

    for (i, path) in paths.enumerate() {
        match move_fs_item(&path, tempdir.path().join(i.to_string())) {
            Ok(_) => {
                moved.push(path.to_path_buf());
            }
            Err(cause) => {
                // NOTE: This dance is to bypass the fact that std::io::error is not Clone-eable...
                let mut catastrophic_error = None;
                for (j, recover) in moved.iter().enumerate() {
                    if let Err(err) = move_fs_item(tempdir.path().join(j.to_string()), recover) {
                        catastrophic_error = Some(err);
                        break;
                    }
                }

                if let Some(err) = catastrophic_error {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                } else {
                    return Err(TryMoveError::RecoveredIO(cause));
                }
            }
        }
    }

    Ok(())
}

// Recursively copy a directory from `src` to `dst`.
// Assumes that all parents of `dst` exist.
fn copy_dir_recursive<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    src: P,
    dst: Q,
) -> Result<(), Box<FsIoError>> {
    wrapfs::create_dir(&dst)?;

    for entry_result in wrapfs::read_dir(&src)? {
        let entry = entry_result.map_err(|e| FsIoError::ReadDir(src.to_path_buf(), e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| FsIoError::ReadDir(src.to_path_buf(), e))?;
        let src_path = entry.path();
        let dst_path = dst.as_ref().join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(src_path, dst_path)?;
        } else {
            wrapfs::copy(src_path, dst_path)?;
        }
    }

    Ok(())
}

// Rename/move a file or directory from `src` to `dst`.
fn move_fs_item<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    src: P,
    dst: Q,
) -> Result<(), Box<FsIoError>> {
    match fs::rename(src.as_ref(), dst.as_ref()) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            let metadata = wrapfs::metadata(&src)?;
            if metadata.is_dir() {
                copy_dir_recursive(&src, &dst)?;
                wrapfs::remove_dir_all(&src)?;
            } else {
                wrapfs::copy(&src, &dst)?;
                wrapfs::remove_file(&src)?;
            }
            Ok(())
        }
        Err(e) => Err(FsIoError::Move(src.to_path_buf(), dst.to_path_buf(), e))?,
    }
}

fn try_move_files(paths: &Vec<(&Utf8Path, &Utf8Path)>) -> Result<(), TryMoveError> {
    let tempdir = camino_tempfile::tempdir()
        .map_err(|e| TryMoveError::RecoveredIO(FsIoError::CreateTempFile(e).into()))?;

    let mut last_err = None;

    // move source files out of the way
    for (i, (path, _)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{}", i));
        if let Err(e) = move_fs_item(path, src_path) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = move_fs_item(src_path, path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    let mut last_err = None;

    // Move target files out of the way
    for (i, (_, path)) in paths.iter().enumerate() {
        if path.exists() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));
            if let Err(e) = move_fs_item(path, trg_path) {
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
                if let Err(err) = move_fs_item(trg_path, path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = move_fs_item(src_path, path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    let mut last_err = None;

    // Try moving files to destination
    for (i, (_, target)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{}", i));

        if let Err(e) = move_fs_item(src_path, target) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (_, path)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if path.exists() {
                if let Err(err) = move_fs_item(path, src_path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));

            if trg_path.exists() {
                if let Err(err) = move_fs_item(trg_path, path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists() {
                if let Err(err) = move_fs_item(src_path, path) {
                    return Err(TryMoveError::CatastrophicIO { err, cause });
                }
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    Ok(())
}

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

fn add_line_temp<R: Read, S: AsRef<str>>(
    reader: R,
    line: S,
) -> Result<NamedUtf8TempFile, LocalWriteError> {
    let mut temp_file = NamedUtf8TempFile::new().map_err(FsIoError::CreateTempFile)?;

    let mut line_added = false;
    for this_line in BufReader::new(reader).lines() {
        let this_line = this_line.map_err(|e| FsIoError::ReadFile(temp_file.to_path_buf(), e))?;

        if !line_added && line.as_ref() < this_line.as_str() {
            writeln!(temp_file, "{}", line.as_ref())
                .map_err(|e| FsIoError::WriteFile(temp_file.path().into(), e))?;
            line_added = true;
        }

        writeln!(temp_file, "{}", this_line)
            .map_err(|e| FsIoError::WriteFile(temp_file.path().into(), e))?;

        if line.as_ref() == this_line {
            line_added = true;
        }
    }

    if !line_added {
        writeln!(temp_file, "{}", line.as_ref())
            .map_err(|e| FsIoError::WriteFile(temp_file.path().into(), e))?;
    }

    Ok(temp_file)
}

fn singleton_line_temp<S: AsRef<str>>(line: S) -> Result<NamedUtf8TempFile, LocalWriteError> {
    let mut temp_file = NamedUtf8TempFile::new().map_err(FsIoError::CreateTempFile)?;

    writeln!(temp_file, "{}", line.as_ref())
        .map_err(|e| FsIoError::WriteFile(temp_file.path().into(), e))?;

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
