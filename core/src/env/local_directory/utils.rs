// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
};

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::NamedUtf8TempFile;
use sha2::Sha256;
use thiserror::Error;

use crate::{
    env::{local_directory::LocalWriteError, segment_uri_generic},
    project::utils::{FsIoError, ToPathBuf, wrapfs},
};

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

pub fn try_remove_files<P: AsRef<Utf8Path>, I: Iterator<Item = P>>(
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

pub fn try_move_files(paths: &Vec<(&Utf8Path, &Utf8Path)>) -> Result<(), TryMoveError> {
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

pub fn add_line_temp<R: Read, S: AsRef<str>>(
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

pub fn singleton_line_temp<S: AsRef<str>>(line: S) -> Result<NamedUtf8TempFile, LocalWriteError> {
    let mut temp_file = NamedUtf8TempFile::new().map_err(FsIoError::CreateTempFile)?;

    writeln!(temp_file, "{}", line.as_ref())
        .map_err(|e| FsIoError::WriteFile(temp_file.path().into(), e))?;

    Ok(temp_file)
}
