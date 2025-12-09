// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("invalid directory: `{0}`")]
    InvalidDirectory(String),
    #[error("unable to find project with IRI `{0}`")]
    NoResolve(String),
    #[error("invalid IRI `{0}`: {1}")]
    InvalidIri(String, fluent_uri::ParseError),
    #[error("unable to find interchange project `{0}`")]
    MissingProject(String),
    #[error("unable to find interchange project `{0}` version {1}")]
    MissingProjectVersion(String, String),
    #[error("unable to find interchange project in current directory")]
    MissingProjectCurrentDir,
}
