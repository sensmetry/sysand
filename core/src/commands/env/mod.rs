// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "filesystem")]
use crate::env::local_directory::{ENTRIES_PATH, LocalDirectoryEnvironment, LocalWriteError};
use crate::env::memory::{MemoryStorageEnvironment, MemoryWriteError};

use thiserror::Error;

mod install;
pub use install::do_env_install_project;

mod uninstall;
pub use uninstall::do_env_uninstall;

mod list;
pub use list::do_env_list;

#[derive(Error, Debug)]
pub enum EnvError<WriteError: std::error::Error> {
    #[error("refusing to overwrite")]
    AlreadyExists(PathBuf),
    #[error("environment write error")]
    WriteError(#[from] WriteError),
}

pub fn do_env_memory() -> Result<MemoryStorageEnvironment, EnvError<MemoryWriteError>> {
    Ok(MemoryStorageEnvironment::default())
}

#[cfg(feature = "filesystem")]
pub fn do_env_local_dir<P: AsRef<Path>>(
    path: P,
) -> Result<LocalDirectoryEnvironment, EnvError<LocalWriteError>> {
    if path.as_ref().exists() {
        return Err(EnvError::AlreadyExists(path.as_ref().to_path_buf()));
    }

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} env");

    std::fs::create_dir(path.as_ref())
        .map_err(|e| EnvError::WriteError(LocalWriteError::IOError(e)))?;

    std::fs::File::create(path.as_ref().join(ENTRIES_PATH))
        .map_err(|e| EnvError::WriteError(LocalWriteError::IOError(e)))?;

    Ok(LocalDirectoryEnvironment {
        environment_path: path.as_ref().to_path_buf(),
    })
}
