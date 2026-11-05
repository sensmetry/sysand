// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

pub mod add;
pub mod auth;
pub mod build;
pub mod clone;
pub mod edit;
pub mod env;
pub mod exclude;
pub mod include;
pub mod index;
pub mod info;
pub mod init;
pub mod lock;
pub mod migrate;
pub mod print_root;
pub mod publish;
pub mod remove;
pub mod sources;
pub mod sync;

use sysand_core::model::KnownMetamodel;

/// Check that a `--metamodel` value the user typed is a recognized
/// metamodel, so the error can name the flag.
pub fn check_metamodel(value: &str) -> anyhow::Result<KnownMetamodel> {
    KnownMetamodel::recognize(value).ok_or_else(|| {
        anyhow::anyhow!(
            "`--metamodel` is not a metamodel Sysand recognizes: `{value}`\n\
            {SP:>8} it is one of {list}",
            SP = ' ',
            list = KnownMetamodel::RECOGNIZED,
        )
    })
}
pub mod versions;
