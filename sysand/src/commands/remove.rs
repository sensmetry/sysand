// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use sysand_core::{project::local_src::LocalSrcProject, remove::do_remove};

use crate::CliError;

pub fn command_remove<S: AsRef<str>>(
    iri: S,
    current_project: Option<LocalSrcProject>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;

    let usages = do_remove(&mut current_project, &iri)?;

    let removed = "Removed";
    let header = sysand_core::style::get_style_config().header;
    if let [usage] = usages.as_slice() {
        match usage.version_constraint {
            Some(ref vc) => {
                log::info!(
                    "{header}{removed:>12}{header:#} `{}` with version constraints `{}`",
                    &usage.resource,
                    vc
                );
            }
            None => {
                log::info!("{header}{removed:>12}{header:#} `{}`", &usage.resource,);
            }
        }
    } else {
        log::info!("{header}{removed:>12}{header:#}:");
        for usage in usages {
            match usage.version_constraint {
                Some(vc) => {
                    log::info!(
                        "{:>13} `{}` with version constraints `{}`",
                        ' ',
                        &usage.resource,
                        vc
                    );
                }
                None => {
                    log::info!("{:>13} `{}`", ' ', &usage.resource,);
                }
            }
        }
    }

    Ok(())
}
