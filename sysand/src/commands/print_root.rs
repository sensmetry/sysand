// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, anyhow};

use camino::Utf8Path;
use sysand_core::root::do_root;

use crate::CliError;

pub fn command_print_root<P: AsRef<Utf8Path>>(path: P) -> Result<()> {
    match do_root(path)? {
        Some(root) => {
            println!("{}", root.canonicalize()?.display());
            Ok(())
        }
        None => Err(anyhow!(CliError::InvalidDirectory(
            "not inside a project".to_string(),
        ))),
    }
}
