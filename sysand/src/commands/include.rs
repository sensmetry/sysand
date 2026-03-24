// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
// use glob::glob;
use sysand_core::{context::ProjectContext, include::do_include, project::utils::wrapfs};

use crate::CliError;

pub fn command_include(
    files: Vec<Utf8PathBuf>,
    compute_checksum: bool,
    index_symbols: bool,
    ctx: ProjectContext,
) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;

    for file in files {
        if !wrapfs::is_file(file.to_path_buf())? {
            bail!("`{file}` does not exist or is not a file");
        }
        let unix_path = current_project.get_unix_path(file)?;

        do_include(
            &mut current_project,
            unix_path,
            compute_checksum,
            index_symbols,
            None,
        )?;
    }

    Ok(())
}
