// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::env::utils::ErrorBound;
#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::{LocalDirectoryEnvironment, LocalWriteError},
    project::utils::wrapfs,
};

#[cfg(feature = "filesystem")]
use camino::Utf8Path;
use camino::Utf8PathBuf;
#[cfg(feature = "filesystem")]
use std::io::ErrorKind;
use thiserror::Error;

mod install;
pub use install::{EnvInstallError, EnvInstallProjectError, do_env_install_project};
#[cfg(feature = "filesystem")]
pub use install::{EnvInstallPathError, do_env_install_path, do_env_install_path_parse};

mod uninstall;
pub use uninstall::do_env_uninstall;

mod list;
pub use list::do_env_list;

#[derive(Error, Debug)]
pub enum EnvError<WriteError: ErrorBound> {
    #[error("refusing to overwrite `{0}`")]
    AlreadyExists(Utf8PathBuf),
    #[error("environment write error")]
    Write(#[from] WriteError),
}

#[cfg(feature = "filesystem")]
pub fn do_env_local_dir<P: AsRef<Utf8Path>>(
    path: P,
) -> Result<LocalDirectoryEnvironment, EnvError<LocalWriteError>> {
    let path = path.as_ref();

    let creating = "Creating";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{creating:>12}{header:#} env");

    // A `.sysand/` holding only the workspace lock file is not an
    // environment yet; `env.toml` is what makes one, and creating it
    // fails if it already exists.
    match wrapfs::create_dir(path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
        Err(err) => return Err(EnvError::Write(LocalWriteError::from(err))),
    }

    LocalDirectoryEnvironment::create(path).map_err(|err| {
        if err.kind() == ErrorKind::AlreadyExists {
            EnvError::AlreadyExists(path.to_path_buf())
        } else {
            EnvError::Write(LocalWriteError::from(err))
        }
    })
}
