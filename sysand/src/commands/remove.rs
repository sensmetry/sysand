// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8PathBuf;

use sysand_core::{
    config::local_fs::{CONFIG_FILE, remove_project_source_from_config},
    project::local_src::LocalSrcProject,
    remove::do_remove,
};

use crate::CliError;

pub fn command_remove<S: AsRef<str>>(
    iri: S,
    current_project: Option<LocalSrcProject>,
    config_file: Option<String>,
    no_config: bool,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;

    let config_path = config_file
        .map(Utf8PathBuf::from)
        .or((!no_config).then(|| current_project.root_path().join(CONFIG_FILE)));

    if let Some(path) = config_path {
        remove_project_source_from_config(path, &iri)?;
    }

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
