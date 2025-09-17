// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
// use glob::glob;
use sysand_core::{include::do_include, project::local_src::LocalSrcProject};

use crate::CliError;

pub fn command_include(
    files: Vec<String>,
    compute_checksum: bool,
    index_symbols: bool,
    current_project: Option<LocalSrcProject>,
) -> Result<()> {
    let mut current_project =
        current_project.ok_or(CliError::MissingProject("in current directory".to_string()))?;

    let including = "Including";
    let header = crate::style::CONFIG.header;
    log::info!("{header}{including:>12}{header:#} files: {:?}", &files,);

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
