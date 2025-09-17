// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(not(feature = "std"))]
compile_error!("`std` feature is currently required to build `sysand`");

use anyhow::{Result, bail};

use sysand_core::{
    config::{
        Config,
        local_fs::{get_config, load_configs},
    },
    env::local_directory::{DEFAULT_ENV_NAME, LocalDirectoryEnvironment},
    project::ProjectRead,
};

use crate::commands::{
    add::command_add,
    build::command_build,
    env::{command_env, command_env_install, command_env_list, command_env_uninstall},
    exclude::command_exclude,
    include::command_include,
    info::command_info_path,
    new::command_new,
    print_root::command_print_root,
    remove::command_remove,
    sources::{command_sources_env, command_sources_project},
    sync::command_sync,
};

pub mod cli;
pub mod commands;
pub mod logger;
pub mod style;

mod error;
pub use error::CliError;

pub fn run_cli(args: cli::Args) -> Result<()> {
    sysand_core::style::set_style_config(crate::style::CONFIG);

    let config = match (&args.global_opts.config_file, &args.global_opts.no_config) {
        (None, false) => Some(load_configs(std::path::Path::new("."))?),
        (None, true) => None,
        (Some(config_path), _) => Some(get_config(std::path::Path::new(config_path))?),
    };

    let (verbose, quiet) = if args.global_opts.sets_log_level() {
        (args.global_opts.verbose, args.global_opts.quiet)
    } else {
        get_config_verbose_quiet(config)
    };
    logger::init(get_log_level(verbose, quiet)?);

    let current_project = sysand_core::discover::current_project()?;

    let project_root = current_project.clone().map(|p| p.root_path()).clone();

    let current_environment = project_root
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .and_then(|p| crate::get_env(&p));

    let client = reqwest::blocking::ClientBuilder::new().build()?;

    match args.command {
        cli::Command::Init { name, version } => {
            command_new(name, version, std::env::current_dir()?)
        }
        cli::Command::New { dir, name, version } => {
            command_new(name, version, std::path::Path::new(&dir))
        }
        cli::Command::Env { command } => match command {
            None => {
                command_env(
                    project_root
                        .unwrap_or(std::env::current_dir()?)
                        .join(DEFAULT_ENV_NAME),
                )?;

                Ok(())
            }
            Some(cli::EnvCommand::Install {
                iri,
                version,
                location,
                index,
                allow_overwrite,
                allow_multiple,
            }) => {
                let mut local_environment = match current_environment {
                    Some(env) => env,
                    None => command_env(
                        project_root
                            .unwrap_or(std::env::current_dir()?)
                            .join(DEFAULT_ENV_NAME),
                    )?,
                };

                command_env_install(
                    iri,
                    version,
                    &mut local_environment,
                    location,
                    index,
                    allow_overwrite,
                    allow_multiple,
                )
            }
            Some(cli::EnvCommand::Uninstall { iri, version }) => match current_environment {
                Some(local_environment) => command_env_uninstall(iri, version, local_environment),
                None => {
                    log::warn!("no environment to uninstall from");
                    Ok(())
                }
            },
            Some(cli::EnvCommand::List) => command_env_list(current_environment),
            Some(cli::EnvCommand::Sources {
                iri,
                version,
                no_deps,
            }) => command_sources_env(iri, version, !no_deps, current_environment),
        },
        cli::Command::Lock { use_index } => {
            if let Some(path) = project_root {
                crate::commands::lock::command_lock(path, client, use_index)
            } else {
                bail!("Not inside a project")
            }
        }
        cli::Command::Sync => {
            let mut local_environment = match current_environment {
                Some(env) => env,
                None => command_env(
                    project_root
                        .as_ref()
                        .unwrap_or(&std::env::current_dir()?)
                        .join(DEFAULT_ENV_NAME),
                )?,
            };

            command_sync(
                project_root.unwrap_or(std::env::current_dir()?),
                &mut local_environment,
                client,
            )
        }
        cli::Command::PrintRoot => command_print_root(std::env::current_dir()?),
        cli::Command::Info {
            path,
            iri,
            auto,
            location,
            no_normalise,
            use_index,
        } => {
            match location {
                Some(actual_location) => {
                    if iri {
                        debug_assert!(!path);
                        debug_assert!(!auto);
                        let uri = fluent_uri::Iri::parse(actual_location.clone())
                            .map_err(|e| CliError::NoResolve(format!("Invalid URI: {}", e)))?;
                        crate::commands::info::command_info_uri(
                            uri,
                            !no_normalise,
                            client,
                            use_index,
                        )
                    } else if auto {
                        debug_assert!(!path);
                        if let Ok(uri) = fluent_uri::Iri::parse(actual_location.clone()) {
                            crate::commands::info::command_info_uri(
                                uri,
                                !no_normalise,
                                client,
                                use_index,
                            )
                        } else {
                            command_info_path(std::path::Path::new(&actual_location))
                        }
                    } else {
                        command_info_path(std::path::Path::new(&actual_location))
                    }
                }
                None => {
                    // TODO: Do project discovery
                    command_info_path(std::path::Path::new("."))
                }
            }
        }
        cli::Command::Add {
            iri,
            versions_constraint,
            no_lock,
            no_sync,
            use_index,
        } => {
            command_add(iri, versions_constraint, current_project)?;

            if !no_lock {
                if let Some(path) = &project_root {
                    crate::commands::lock::command_lock(path, client.clone(), use_index)?;
                } else {
                    bail!("Not inside a project")
                }

                if !no_sync {
                    // TODO: Deduplicate this code
                    let mut local_environment = match current_environment {
                        Some(env) => env,
                        None => command_env(
                            project_root
                                .as_ref()
                                .unwrap_or(&std::env::current_dir()?)
                                .join(DEFAULT_ENV_NAME),
                        )?,
                    };

                    command_sync(
                        project_root.unwrap_or(std::env::current_dir()?),
                        &mut local_environment,
                        client,
                    )?;
                }
            }

            Ok(())
        }
        cli::Command::Remove { iri } => command_remove(iri, current_project),
        cli::Command::Include {
            paths,
            compute_checksum: add_checksum,
            no_index_symbols,
        } => command_include(paths, add_checksum, !no_index_symbols, current_project),
        cli::Command::Exclude { paths } => command_exclude(paths, current_project),
        cli::Command::Build { path } => {
            let current_project = current_project
                .ok_or(CliError::MissingProject("in current directory".to_string()))?;

            let path = if let Some(path) = path {
                path
            } else {
                let output_dir = current_project.project_path.join("output");
                if !output_dir.is_dir() {
                    std::fs::create_dir(&output_dir)?;
                }
                let name = current_project.name()?.unwrap_or("project".to_string());
                output_dir.join(format!("{}.kpar", name))
            };

            command_build(path, current_project)
        }
        cli::Command::Sources { no_deps } => {
            command_sources_project(!no_deps, current_project, current_environment)
        }
    }
}

pub fn get_env(project_root: &std::path::Path) -> Option<LocalDirectoryEnvironment> {
    let environment_path = project_root.join(DEFAULT_ENV_NAME);
    if !environment_path.is_dir() {
        None
    } else {
        Some(LocalDirectoryEnvironment { environment_path })
    }
}

fn get_config_verbose_quiet(config: Option<Config>) -> (bool, bool) {
    config
        .map(|config| {
            (
                config.verbose.unwrap_or_default(),
                config.quiet.unwrap_or_default(),
            )
        })
        .unwrap_or((false, false))
}

fn get_log_level(verbose: bool, quiet: bool) -> Result<log::LevelFilter> {
    match (verbose, quiet) {
        (true, true) => unreachable!(),
        (true, false) => Ok(log::LevelFilter::Debug),
        (false, true) => Ok(log::LevelFilter::Error),
        (false, false) => Ok(log::LevelFilter::Info),
    }
}
