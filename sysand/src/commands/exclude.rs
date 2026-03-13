// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use sysand_core::{context::ProjectContext, exclude::do_exclude};

use crate::CliError;

pub fn command_exclude(paths: Vec<String>, ctx: ProjectContext) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;

    for path in paths {
        let unix_path = current_project.get_unix_path(&path)?;

        do_exclude(&mut current_project, unix_path)?;
    }

    Ok(())
}
