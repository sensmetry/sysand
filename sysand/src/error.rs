// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("invalid directory: {0}")]
    InvalidDirectory(String),
    #[error("invalid path: {0}")]
    InvalidPath(PathBuf),
    #[error("cannot handle IRI/URL: {0}")]
    NoResolve(String),
    #[error("unable to find interchange project: {0}")]
    MissingProject(String),
    #[error("unable to find local project environment: {0}")]
    MissingEnvironment(String),
}
