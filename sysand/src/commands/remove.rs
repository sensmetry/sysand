// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::{Result, bail};
use sysand_core::{project::local_src::LocalSrcProject, remove::do_remove};

use crate::CliError;

pub fn command_remove(iri: String, current_project: Option<LocalSrcProject>) -> Result<()> {
    let mut current_project =
        current_project.ok_or(CliError::MissingProject("in current directory".to_string()))?;

    let usages = do_remove(&mut current_project, &iri)?;

    if usages.is_empty() {
        bail!("could not find usage for {}", iri);
    }

    let removing = "Removed";
    let header = crate::style::CONFIG.header;
    log::info!("{header}{removing:>12}{header:#} usages:",);
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
