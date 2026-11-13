// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::Result;
use camino::Utf8PathBuf;
use sysand_core::{context::ProjectContext, exclude::do_exclude};

pub fn command_exclude(paths: Vec<Utf8PathBuf>, mut ctx: ProjectContext) -> Result<()> {
    let current_project = ctx
        .current_project()
        .expect("BUG: the command needs a project, and its context has none");

    let mut unix_paths = Vec::with_capacity(paths.len());
    for f in paths {
        unix_paths.push(current_project.get_unix_path(f)?);
    }

    ctx.edit_project(|project| do_exclude(project, unix_paths.into_iter()))?;

    Ok(())
}
