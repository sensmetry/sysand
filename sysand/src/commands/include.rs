// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{context::ProjectContext, include::do_include, project::utils::wrapfs};

use crate::CliError;

pub fn command_include(
    paths: Vec<Utf8PathBuf>,
    compute_checksum: bool,
    index_symbols: bool,
    ctx: ProjectContext,
) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;

    let mut unix_paths = Vec::with_capacity(paths.len());
    for p in paths {
        if !wrapfs::is_file(&p)? {
            bail!("`{p}` does not exist or is not a file");
        }
        unix_paths.push(current_project.get_unix_path(p)?);
    }
    do_include(
        &mut current_project,
        unix_paths.into_iter(),
        compute_checksum,
        index_symbols,
        None,
    )?;

    Ok(())
}
