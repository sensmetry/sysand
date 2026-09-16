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

/// The four recognized metamodels (model-toml.md "Metamodel"), checked
/// on a value the user typed rather than a value a manifest holds.
///
/// `init` and `migrate` both take `--metamodel`, and both check it
/// before anything is written: the core commands check it too, so that
/// a binding caller is not held to a lower standard, but only here is
/// there a flag to name.
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
