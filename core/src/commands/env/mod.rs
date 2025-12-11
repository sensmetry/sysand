// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "filesystem")]
use std::path::Path;
use std::path::PathBuf;

use crate::env::{
    memory::{MemoryStorageEnvironment, MemoryWriteError},
    utils::ErrorBound,
};
#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::{ENTRIES_PATH, LocalDirectoryEnvironment, LocalWriteError},
    project::utils::wrapfs,
};

use thiserror::Error;

mod install;
pub use install::do_env_install_project;

mod uninstall;
pub use uninstall::do_env_uninstall;

mod list;
pub use list::do_env_list;

#[derive(Error, Debug)]
pub enum EnvError<WriteError: ErrorBound> {
    #[error("refusing to overwrite `{0}`")]
    AlreadyExists(PathBuf),
    #[error("environment write error: {0}")]
    Write(#[from] WriteError),
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

    wrapfs::create_dir(path.as_ref()).map_err(LocalWriteError::from)?;

    let fp = path.as_ref().join(ENTRIES_PATH);
    wrapfs::File::create(&fp).map_err(LocalWriteError::from)?;

    Ok(LocalDirectoryEnvironment {
        environment_path: path.as_ref().to_path_buf(),
    })
}
