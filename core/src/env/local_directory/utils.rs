// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{fs, io};

use camino::Utf8Path;
use thiserror::Error;

use crate::project::utils::{FsIoError, ToPathBuf as _, wrapfs};

/// Removes all files in the directory.
/// All errors are ignored, but logged with `log::warn!()`.
pub fn clean_dir<P: AsRef<Utf8Path>>(path: P) {
    let path = path.as_ref();
    let entries = match path.read_dir_utf8() {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("failed to read directory entries for `{path}`: {e}");
            return;
        }
    };
    log::debug!("clearing contents of dir `{path}`");

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("failed to read a dir entry of `{path}`: {e}");
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(entry_type) => {
                if entry_type.is_dir() {
                    clean_dir(path);
                    if let Err(e) = fs::remove_dir(path) {
                        log::warn!("failed to remove empty dir `{path}`: {e}");
                    }
                } else if let Err(e) = fs::remove_file(path) {
                    log::warn!("failed to remove file/symlink `{path}`: {e}");
                }
            }
            Err(e) => {
                log::warn!("failed to get file type of `{path}`: {e}");
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum TryMoveError {
    #[error("recovered from failure")]
    RecoveredIO(#[source] Box<FsIoError>),
    #[error("failed and may have left the directory in inconsistent state:\n{err}")]
    CatastrophicIO {
        err: Box<FsIoError>,
        source: Box<FsIoError>,
    },
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
pub(crate) fn move_fs_item<P: AsRef<Utf8Path>, Q: AsRef<Utf8Path>>(
    src: P,
    dst: Q,
) -> Result<(), Box<FsIoError>> {
    match fs::rename(src.as_ref(), dst.as_ref()) {
        Ok(()) => Ok(()),
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
        Err(e) => Err(FsIoError::Move(src.to_path_buf(), dst.to_path_buf(), e).into()),
    }
}

/// Try moving paths from sources (`paths[].0`) to destinations (`paths[].1`). Sources can
/// overlap with destinations and it won't cause loss of any source contents, as all sources
/// are moved to a separate location before attempting any moves to destinations.
/// Contents initially present at the destinations will first be moved out of the way,
/// and if all moves to the destinations succeed, will be deleted.
/// On any failures an attempt is made to restore all source and destination paths to
/// their original contents.
pub fn try_move_files(paths: &[(&Utf8Path, &Utf8Path)]) -> Result<(), TryMoveError> {
    let tempdir = camino_tempfile::tempdir()
        .map_err(|e| TryMoveError::RecoveredIO(FsIoError::CreateTempFile(e).into()))?;

    let mut last_err = None;

    // move source files out of the way to not overwrite them in case some dest overlaps
    // with any source file
    for (i, (path, _)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{i}"));
        if let Err(e) = move_fs_item(path, src_path) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{i}"));

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    let mut last_err = None;

    // Move target files out of the way
    for (i, (_, path)) in paths.iter().enumerate() {
        if path.exists() {
            let trg_path = tempdir.path().join(format!("trg_{i}"));
            if let Err(e) = move_fs_item(path, trg_path) {
                last_err = Some(e);
                break;
            }
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{i}"));

            if trg_path.exists()
                && let Err(err) = move_fs_item(trg_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{i}"));

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    let mut last_err = None;

    // Try moving files to destination
    for (i, (_, target)) in paths.iter().enumerate() {
        let src_path = tempdir.path().join(format!("src_{i}"));

        if let Err(e) = move_fs_item(src_path, target) {
            last_err = Some(e);
            break;
        }
    }

    // Recover moved files in case of failure
    if let Some(cause) = last_err {
        for (i, (_, path)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{i}"));

            if path.exists()
                && let Err(err) = move_fs_item(path, src_path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{i}"));

            if trg_path.exists()
                && let Err(err) = move_fs_item(trg_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{i}"));

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, source: cause });
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    Ok(())
}
