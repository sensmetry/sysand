// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The hints printed beneath an error whose core message names no command.
//!
//! The library states the condition and the frontend owns the command
//! vocabulary (the rule `publish_error_with_hint` follows for `sysand auth`).
//! `program` is how the user spells this command line (`sysand`, or an
//! embedder's own name), so what a hint says to run is what the user can
//! type.

use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{
        DEFAULT_ENV_NAME,
        local_directory::{
            METADATA_PATH,
            metadata::{EnvMetadataError, ParseError as EnvParseError},
        },
    },
    lock::{CURRENT_LOCK_VERSION, LockfileError, ParseError as LockParseError, VersionError},
};

/// A way out of a condition, printed beneath its error once the program's
/// name is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hint {
    NewerLockfile,
    NewerEnvironment,
    MissingLockfile,
}

impl Hint {
    /// The way out of a lockfile that could not be read, if there is one.
    pub(crate) fn for_lockfile(error: &LockfileError) -> Option<Self> {
        match error {
            LockfileError::Parse(_, LockParseError::Version(VersionError::Newer(_))) => {
                Some(Self::NewerLockfile)
            }
            LockfileError::Missing(_) => Some(Self::MissingLockfile),
            _ => None,
        }
    }

    /// The way out of an environment that could not be read, if there is one.
    pub(crate) fn for_environment(error: &EnvMetadataError) -> Option<Self> {
        match error {
            EnvMetadataError::Parse(_, EnvParseError::NewerVersion(_)) => {
                Some(Self::NewerEnvironment)
            }
            _ => None,
        }
    }

    /// The hint text, with `program` as the way the user spells this
    /// command line.
    pub(crate) fn render(self, program: &str) -> String {
        match self {
            Self::NewerLockfile => newer_lockfile(program),
            Self::NewerEnvironment => newer_environment(program),
            Self::MissingLockfile => format!("create it with `{program} lock` or `{program} sync`"),
        }
    }
}

/// A lockfile a newer release wrote. Regenerating it is possible, but as the
/// lockfile is shared through version control, that reverts it for everyone
/// on the newer release, so upgrading comes first and the cost is spelled
/// out.
fn newer_lockfile(program: &str) -> String {
    format!(
        "upgrade to the release that wrote `{DEFAULT_LOCKFILE_NAME}`; or regenerate it with\n\
         `{program} lock`, which writes version `{CURRENT_LOCK_VERSION}` and reverts the file for\n\
         everyone who shares it"
    )
}

/// An environment a newer release wrote. It is local and rebuilt from the
/// lockfile, so recreating it costs nothing, but only the whole directory
/// can go: `sync` refuses to overwrite a `{DEFAULT_ENV_NAME}` directory it
/// cannot read.
fn newer_environment(program: &str) -> String {
    format!(
        "upgrade to the release that wrote `{DEFAULT_ENV_NAME}/{METADATA_PATH}`; or delete the\n\
         `{DEFAULT_ENV_NAME}` directory and run `{program} sync` to recreate it for this release"
    )
}

#[cfg(test)]
#[path = "./hints_tests.rs"]
mod tests;
