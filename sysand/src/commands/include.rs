// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8PathBuf;
// use glob::glob;
use sysand_core::{include::do_include, project::local_src::LocalSrcProject};

use crate::CliError;

pub fn command_include(
    files: Vec<Utf8PathBuf>,
    compute_checksum: bool,
    index_symbols: bool,
    current_project: Option<LocalSrcProject>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;

    for file in files {
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
