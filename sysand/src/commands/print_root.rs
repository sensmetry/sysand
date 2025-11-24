// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use anyhow::{Result, anyhow};

use sysand_core::root::do_root;

use crate::CliError;

pub fn command_print_root<P: AsRef<Path>>(path: P) -> Result<()> {
    match do_root(path) {
        Some(root) => {
            println!("{}", root.canonicalize()?.display());
            Ok(())
        }
        None => Err(anyhow!(CliError::InvalidDirectory(
            "not inside a project".to_string(),
        ))),
    }
}
