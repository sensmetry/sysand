// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    fs,
    io::{self},
};

use camino::Utf8Path;
use thiserror::Error;

use crate::project::utils::{FsIoError, ToPathBuf, wrapfs};

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
                    let _ = fs::remove_dir(path)
                        .map_err(|e| log::warn!("failed to remove empty dir `{path}`: {e}"));
                } else {
                    let _ = fs::remove_file(path)
                        .map_err(|e| log::warn!("failed to remove file/symlink `{path}`: {e}"));
                };
            }
            Err(e) => {
                log::warn!("failed to get file type of `{path}`: {e}");
            }
        }
    }
}

// pub fn remove_dir_if_empty<P: AsRef<Utf8Path>>(path: P) -> Result<(), FsIoError> {
//     match fs::remove_dir(path.as_ref()) {
//         Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(()),
//         r => r.map_err(|e| FsIoError::RmDir(path.to_path_buf(), e)),
//     }
// }

// pub fn remove_empty_dirs<P: AsRef<Utf8Path>>(path: P) -> Result<(), FsIoError> {
//     let mut dirs: Vec<_> = walkdir::WalkDir::new(path.as_ref())
//         .into_iter()
//         .filter_map(|e| e.ok())
//         .filter_map(|e| {
//             e.file_type()
//                 .is_dir()
//                 .then(|| Utf8PathBuf::from_path_buf(e.into_path()).ok())
//                 .flatten()
//         })
//         .collect();

//     dirs.sort_by(|a, b| b.cmp(a));

//     for dir in dirs {
//         remove_dir_if_empty(&dir)?;
//     }

//     Ok(())
// }

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

// pub fn try_remove_files<P: AsRef<Utf8Path>, I: Iterator<Item = P>>(
//     paths: I,
// ) -> Result<(), TryMoveError> {
//     let tempdir = camino_tempfile::tempdir()
//         .map_err(|e| TryMoveError::RecoveredIO(FsIoError::CreateTempFile(e).into()))?;
//     let mut moved: Vec<Utf8PathBuf> = vec![];

//     for (i, path) in paths.enumerate() {
//         match move_fs_item(&path, tempdir.path().join(i.to_string())) {
//             Ok(_) => {
//                 moved.push(path.to_path_buf());
//             }
//             Err(cause) => {
//                 // NOTE: This dance is to bypass the fact that std::io::error is not cloneable...
//                 let mut catastrophic_error = None;
//                 for (j, recover) in moved.iter().enumerate() {
//                     if let Err(err) = move_fs_item(tempdir.path().join(j.to_string()), recover) {
//                         catastrophic_error = Some(err);
//                         break;
//                     }
//                 }

//                 if let Some(err) = catastrophic_error {
//                     return Err(TryMoveError::CatastrophicIO { err, cause });
//                 } else {
//                     return Err(TryMoveError::RecoveredIO(cause));
//                 }
//             }
//         }
//     }

//     Ok(())
// }

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

pub fn try_move_files(paths: &[(&Utf8Path, &Utf8Path)]) -> Result<(), TryMoveError> {
    let tempdir = camino_tempfile::tempdir()
        .map_err(|e| TryMoveError::RecoveredIO(FsIoError::CreateTempFile(e).into()))?;

    let mut last_err = None;

    // move source files out of the way
    // TODO: why is this needed?
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

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
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

            if trg_path.exists()
                && let Err(err) = move_fs_item(trg_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
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

            if path.exists()
                && let Err(err) = move_fs_item(path, src_path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
            }
        }

        for (i, (_, path)) in paths.iter().enumerate() {
            let trg_path = tempdir.path().join(format!("trg_{}", i));

            if trg_path.exists()
                && let Err(err) = move_fs_item(trg_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
            }
        }

        for (i, (path, _)) in paths.iter().enumerate() {
            let src_path = tempdir.path().join(format!("src_{}", i));

            if src_path.exists()
                && let Err(err) = move_fs_item(src_path, path)
            {
                return Err(TryMoveError::CatastrophicIO { err, cause });
            }
        }

        return Err(TryMoveError::RecoveredIO(cause));
    }

    Ok(())
}
