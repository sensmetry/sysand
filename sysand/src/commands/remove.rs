// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8PathBuf;

use sysand_core::{
    config::local_fs::{CONFIG_FILE, remove_project_source_from_config},
    context::ProjectContext,
    model::InterchangeProjectUsageG,
    remove::{do_remove, do_remove_experimental},
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

    // TODO: this is trickier, the project may appear as a transitive dep,
    // so it's not always correct to remove the override
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
            _ => (),
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
                _ => (),
            }
        }
    }

    Ok(())
}

pub fn command_remove_experimental(
    publisher: String,
    name: String,
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

    // if let Some(path) = config_path {
    //     remove_project_source_from_config(path, &iri)?;
    // }

    let usages = do_remove_experimental(&mut current_project, &publisher, &name)?;

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
            _ => (),
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
                _ => (),
            }
        }
    }

    Ok(())
}
