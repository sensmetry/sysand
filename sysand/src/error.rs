// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("invalid directory: '{0}'")]
    InvalidDirectory(String),
    #[error("cannot handle IRI/URL: {0}")]
    NoResolve(String),
    #[error("unable to find interchange project '{0}'")]
    MissingProject(String),
    #[error("unable to find interchange project in current directory")]
    MissingProjectCurrentDir,
}
