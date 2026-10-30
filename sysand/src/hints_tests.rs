// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::str::FromStr as _;

use anyhow::Context as _;
use camino::Utf8PathBuf;
use sysand_core::{
    env::local_directory::metadata::{EnvMetadataError, ParseError as EnvParseError},
    lock::Lock,
};

use super::for_error;

/// The lockfile error as `sync` raises it: the path as context, the parse
/// error beneath.
fn lockfile_error(lock_version: &str) -> anyhow::Error {
    Lock::from_str(&format!("lock_version = \"{lock_version}\"\n"))
        .map(|_| ())
        .context("invalid lockfile `sysand-lock.toml`")
        .unwrap_err()
}

#[test]
fn a_lockfile_a_newer_release_wrote_names_the_program_s_lock_command() {
    let hint = for_error(&lockfile_error("99.0"), "syside project").unwrap();
    assert!(
        hint.starts_with("upgrade to the release that wrote `sysand-lock.toml`"),
        "{hint}"
    );
    assert!(hint.contains("`syside project lock`"), "{hint}");
    assert!(hint.contains("reverts the file"), "{hint}");
}

#[test]
fn an_older_or_unknown_lockfile_version_has_no_hint() {
    assert_eq!(for_error(&lockfile_error("0.3"), "sysand"), None);
    assert_eq!(for_error(&lockfile_error("X"), "sysand"), None);
}

#[test]
fn an_environment_a_newer_release_wrote_names_the_program_s_sync_command() {
    let err = anyhow::Error::new(EnvMetadataError::Parse(
        Utf8PathBuf::from(".sysand/env.toml"),
        EnvParseError::NewerVersion("99.0".to_owned()),
    ));
    let hint = for_error(&err, "syside project").unwrap();
    assert!(
        hint.starts_with("upgrade to the release that wrote `.sysand/env.toml`"),
        "{hint}"
    );
    assert!(
        hint.contains("delete the\n`.sysand` directory and run `syside project sync`"),
        "{hint}"
    );
}

#[test]
fn an_unreadable_environment_of_this_version_has_no_hint() {
    let err = anyhow::Error::new(EnvMetadataError::Parse(
        Utf8PathBuf::from(".sysand/env.toml"),
        EnvParseError::UnsupportedVersion("x.y".to_owned()),
    ));
    assert_eq!(for_error(&err, "sysand"), None);
}

#[test]
fn an_unrelated_error_has_no_hint() {
    assert_eq!(
        for_error(&anyhow::anyhow!("something else"), "sysand"),
        None
    );
}
