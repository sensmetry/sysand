// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{context::ProjectContext, include::do_include, project::utils::wrapfs};

pub fn command_include(
    paths: Vec<Utf8PathBuf>,
    compute_checksum: bool,
    index_symbols: bool,
    mut ctx: ProjectContext,
) -> Result<()> {
    let current_project = ctx
        .current_project()
        .expect("BUG: the command needs a project, and its context has none");

    let mut unix_paths = Vec::with_capacity(paths.len());
    for p in paths {
        if !wrapfs::is_file(&p)? {
            bail!("`{p}` does not exist or is not a file");
        }
        unix_paths.push(current_project.get_unix_path(p)?);
    }
    ctx.edit_project(|project| {
        do_include(
            project,
            unix_paths.into_iter(),
            compute_checksum,
            index_symbols,
            None,
        )
    })?;

    Ok(())
}
