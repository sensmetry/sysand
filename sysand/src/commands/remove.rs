// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8PathBuf;

use sysand_core::{
    config::local_fs::{CONFIG_FILE, remove_project_source_from_config},
    context::ProjectContext,
    model::InterchangeProjectUsageG,
    remove::do_remove,
};

use crate::CliError;

pub fn command_remove<S: AsRef<str>>(
    iri: S,
    ctx: ProjectContext,
    config_file: Option<String>,
    no_config: bool,
) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;

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
        match usage {
            InterchangeProjectUsageG::Resource {
                resource,
                version_constraint,
            } => match version_constraint {
                Some(vc) => {
                    log::info!(
                        "{header}{removed:>12}{header:#} `{}` with version constraints `{}`",
                        resource,
                        vc
                    );
                }
                None => {
                    log::info!("{header}{removed:>12}{header:#} `{}`", resource,);
                }
            },
            InterchangeProjectUsageG::Url {
                url,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsageG::Path {
                path,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsageG::Git {
                git,
                id,
                publisher,
                name,
            } => todo!(),
            InterchangeProjectUsageG::Index {
                publisher,
                name,
                version_constraint,
            } => todo!(),
        }
    } else {
        log::info!("{header}{removed:>12}{header:#}:");
        for usage in usages {
            match usage {
                InterchangeProjectUsageG::Resource {
                    resource,
                    version_constraint,
                } => match version_constraint {
                    Some(vc) => {
                        log::info!(
                            "{:>13} `{}` with version constraints `{}`",
                            ' ',
                            resource,
                            vc
                        );
                    }
                    None => {
                        log::info!("{:>13} `{}`", ' ', resource,);
                    }
                },

                InterchangeProjectUsageG::Url {
                    url,
                    publisher,
                    name,
                } => todo!(),
                InterchangeProjectUsageG::Path {
                    path,
                    publisher,
                    name,
                } => todo!(),
                InterchangeProjectUsageG::Git {
                    git,
                    id,
                    publisher,
                    name,
                } => todo!(),
                InterchangeProjectUsageG::Index {
                    publisher,
                    name,
                    version_constraint,
                } => todo!(),
            }
        }
    }

    Ok(())
}
