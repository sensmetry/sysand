// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use camino::Utf8PathBuf;

#[cfg(feature = "filesystem")]
use crate::{
    env::local_directory::LocalDirectoryEnvironment, project::local_src::LocalSrcProject,
    workspace::Workspace,
};

#[derive(Debug, Default)]
pub struct ProjectContext {
    /// Current workspace if found
    #[cfg(feature = "filesystem")]
    pub current_workspace: Option<Workspace>,
    /// Current project if found
    #[cfg(feature = "filesystem")]
    pub current_project: Option<LocalSrcProject>,
    /// Path to current directory
    #[cfg(feature = "filesystem")]
    pub current_directory: Utf8PathBuf,
    /// Metadata of the current workspace/project `sysand_env`. `Some` if
    /// either `current_workspace` or `current_project` is `Some` and
    /// the environment metadata file exists.
    #[cfg(feature = "filesystem")]
    pub env: Option<LocalDirectoryEnvironment>,
}
