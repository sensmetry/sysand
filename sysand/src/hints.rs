// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The hints printed beneath an error whose core message names no command.
//!
//! The library states the condition and the frontend owns the command
//! vocabulary (the rule `publish_error_with_hint` follows for `sysand auth`).
//! `program` is how the user spells this command line -- `sysand`, or an
//! embedder's own name -- so what a hint says to run is what the user can
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
    lock::{CURRENT_LOCK_VERSION, ParseError as LockParseError, VersionError},
};

/// The way out of `err`, if this frontend knows one: the first cause on the
/// chain that has a hint decides.
pub(crate) fn for_error(err: &anyhow::Error, program: &str) -> Option<String> {
    err.chain().find_map(|cause| {
        if let Some(LockParseError::Version(VersionError::Newer(_))) =
            cause.downcast_ref::<LockParseError>()
        {
            Some(newer_lockfile(program))
        } else if let Some(EnvMetadataError::Parse(_, EnvParseError::NewerVersion(_))) =
            cause.downcast_ref::<EnvMetadataError>()
        {
            Some(newer_environment(program))
        } else {
            None
        }
    })
}

/// A lockfile a newer release wrote. Regenerating it is possible, but as the
/// lockfile is shared through version control, that reverts it for everyone
/// on the newer release -- so upgrading comes first and the cost is spelled
/// out.
fn newer_lockfile(program: &str) -> String {
    format!(
        "upgrade to the release that wrote `{DEFAULT_LOCKFILE_NAME}`; or regenerate it with\n\
         `{program} lock`, which writes version `{CURRENT_LOCK_VERSION}` and reverts the file for\n\
         everyone who shares it"
    )
}

/// An environment a newer release wrote. It is local and rebuilt from the
/// lockfile, so recreating it costs nothing -- but only the whole directory
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
