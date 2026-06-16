// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::Result;
use camino::Utf8PathBuf;
use sysand_core::{context::ProjectContext, exclude::do_exclude};

use crate::CliError;

pub fn command_exclude(paths: Vec<Utf8PathBuf>, ctx: ProjectContext) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;

    let mut unix_paths = Vec::with_capacity(paths.len());
    for f in paths {
        unix_paths.push(current_project.get_unix_path(f)?);
    }

    do_exclude(&mut current_project, unix_paths.into_iter())?;

    Ok(())
}
