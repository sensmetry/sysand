// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use sysand_core::{project::local_src::LocalSrcProject, remove::do_remove};

use crate::CliError;

pub fn command_remove(iri: String, current_project: Option<LocalSrcProject>) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;

    let usages = do_remove(&mut current_project, &iri)?;

    for usage in usages {
        log::info!(
            "{:>12} {} {}",
            "",
            &usage.resource,
            usage
                .version_constraint
                .map(|vr| vr.to_string())
                .unwrap_or("".to_string()),
        );
    }

    Ok(())
}
