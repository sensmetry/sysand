// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use anyhow::Result;
use sysand_core::{exclude::do_exclude, project::local_src::LocalSrcProject};

use crate::CliError;

pub fn command_exclude(paths: Vec<String>, current_project: Option<LocalSrcProject>) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;

    for path in paths {
        let path = PathBuf::from(path);
        let unix_path = current_project.get_unix_path(&path)?;

        do_exclude(&mut current_project, unix_path)?;
    }

    Ok(())
}
