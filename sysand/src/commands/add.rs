// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use sysand_core::{add::do_add, project::local_src::LocalSrcProject};

use crate::CliError;

pub fn command_add(
    iri: String,
    versions_constraint: Option<String>,
    current_project: Option<LocalSrcProject>,
) -> Result<()> {
    let mut current_project =
        current_project.ok_or(CliError::MissingProject("in current directory".to_string()))?;

    let adding = "Adding";
    let header = crate::style::CONFIG.header;
    log::info!(
        "{header}{adding:>12}{header:#} usage: {} {}",
        &iri,
        versions_constraint
            .as_ref()
            .map(|vr| vr.to_string())
            .unwrap_or("".to_string()),
    );

    do_add(&mut current_project, iri, versions_constraint)?;

    Ok(())
}
