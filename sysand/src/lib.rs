// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(not(feature = "std"))]
compile_error!("`std` feature is currently required to build `sysand`");

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Result, bail};

use sysand_core::{
    config::{
        Config,
        local_fs::{get_config, load_configs},
    },
    env::local_directory::{DEFAULT_ENV_NAME, LocalDirectoryEnvironment},
    lock::Lock,
    stdlib::known_std_libs,
};

use crate::commands::{
    add::command_add,
    build::{command_build_for_project, command_build_for_workspace},
    env::{
        command_env, command_env_install, command_env_install_path, command_env_list,
        command_env_uninstall,
    },
    exclude::command_exclude,
    include::command_include,
    info::{command_info_current_project, command_info_path, command_info_verb_path},
    lock::command_lock,
    new::command_new,
    print_root::command_print_root,
    remove::command_remove,
    sources::{command_sources_env, command_sources_project},
    sync::command_sync,
};

pub const DEFAULT_INDEX_URL: &str = "https://beta.sysand.org";

pub mod cli;
pub mod commands;
pub mod logger;
pub mod style;

mod error;
pub use error::CliError;

pub fn run_cli(args: cli::Args) -> Result<()> {
    sysand_core::style::set_style_config(crate::style::CONFIG);

    let config = match (&args.global_opts.config_file, &args.global_opts.no_config) {
        (Some(config_path), _) => get_config(Path::new(config_path))?,
        (None, false) => load_configs(Path::new("."))?,
        (None, true) => Config::default(),
    };

    let (verbose, quiet) = if args.global_opts.sets_log_level() {
        (args.global_opts.verbose, args.global_opts.quiet)
    } else {
        get_config_verbose_quiet(&config)
    };
    logger::init(get_log_level(verbose, quiet)?);

    let current_workspace = sysand_core::discover::current_workspace()?;
    let current_project = sysand_core::discover::current_project()?;

    let project_root = current_project.clone().map(|p| p.root_path()).clone();

    let current_environment = project_root
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .and_then(|p| crate::get_env(&p));

    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap(),
    );

    let _runtime_keepalive = runtime.clone();

    match args.command {
        cli::Command::Init { name, version } => {
            command_new(name, version, std::env::current_dir()?)
        }
        cli::Command::New {
            path,
            name,
            version,
        } => command_new(name, version, Path::new(&path)),
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
                path,
                install_opts,
                dependency_opts,
            }) => {
                if let Some(path) = path {
                    command_env_install_path(
                        iri,
                        version,
                        path,
                        install_opts,
                        dependency_opts,
                        &config,
                        project_root,
                        client,
                        runtime,
                    )
                } else {
                    command_env_install(
                        iri,
                        version,
                        install_opts,
                        dependency_opts,
                        &config,
                        project_root,
                        client,
                        runtime,
                    )
                }
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
                sources_opts,
            }) => {
                let cli::SourcesOptions {
                    no_deps,
                    include_std,
                } = sources_opts;
                let provided_iris = if !include_std {
                    known_std_libs()
                } else {
                    HashMap::default()
                };

                command_sources_env(
                    iri,
                    version,
                    !no_deps,
                    current_environment,
                    &provided_iris,
                    include_std,
                )
            }
        },
        cli::Command::Lock { dependency_opts } => {
            if project_root.is_some() {
                crate::commands::lock::command_lock(
                    PathBuf::from("."),
                    dependency_opts,
                    &config,
                    client,
                    runtime,
                )
            } else {
                bail!("Not inside a project")
            }
        }
        cli::Command::Sync { dependency_opts } => {
            let cli::DependencyOptions { include_std, .. } = dependency_opts.clone();
            let mut local_environment = match current_environment {
                Some(env) => env,
                None => command_env(
                    project_root
                        .as_ref()
                        .unwrap_or(&std::env::current_dir()?)
                        .join(DEFAULT_ENV_NAME),
                )?,
            };

            let provided_iris = if !include_std {
                crate::logger::warn_std_deps();
                known_std_libs()
            } else {
                HashMap::default()
            };
            let project_root = project_root.unwrap_or(std::env::current_dir()?);
            let lockfile = project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);
            if !lockfile.is_file() {
                command_lock(
                    PathBuf::from("."),
                    dependency_opts,
                    &config,
                    client.clone(),
                    runtime.clone(),
                )?;
            }
            let lock = Lock::from_str(&std::fs::read_to_string(
                project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME),
            )?)?;
            command_sync(
                lock,
                project_root,
                &mut local_environment,
                client,
                &provided_iris,
                runtime,
            )
        }
        cli::Command::PrintRoot => command_print_root(std::env::current_dir()?),
        cli::Command::Info {
            path,
            iri,
            auto_location,
            no_normalise,
            dependency_opts,
            subcommand,
        } => {
            let cli::DependencyOptions {
                index,
                default_index,
                no_index,
                include_std,
            } = dependency_opts;
            let index_urls = if no_index {
                None
            } else {
                Some(config.index_urls(
                    index,
                    vec![DEFAULT_INDEX_URL.to_string()],
                    default_index,
                )?)
            };
            let excluded_iris: HashSet<_> = if !include_std {
                crate::logger::warn_std_deps();
                known_std_libs().keys().cloned().collect()
            } else {
                HashSet::default()
            };

            enum Location {
                WorkDir,
                Iri(fluent_uri::Iri<String>),
                Path(String),
            }

            let location = if let Some(auto_location) = auto_location {
                debug_assert!(path.is_none());
                debug_assert!(iri.is_none());

                if let Ok(iri) = fluent_uri::Iri::parse(auto_location.clone()) {
                    Location::Iri(iri)
                } else {
                    Location::Path(auto_location)
                }
            } else if let Some(path) = path {
                debug_assert!(auto_location.is_none());
                debug_assert!(iri.is_none());

                Location::Path(path)
            } else if let Some(iri) = iri {
                debug_assert!(path.is_none());
                debug_assert!(auto_location.is_none());

                Location::Iri(fluent_uri::Iri::parse(iri).map_err(|e| {
                    CliError::NoResolve(format!("invalid URI '{}': {}", e.clone().into_input(), e))
                })?)
            } else {
                Location::WorkDir
            };

            match (location, subcommand) {
                (Location::WorkDir, subcommand) => {
                    if let Some(current_project) = sysand_core::discover::current_project()? {
                        match subcommand {
                            Some(subcommand) => {
                                let numbered = subcommand.numbered();
                                command_info_current_project(
                                    current_project,
                                    subcommand.as_verb(),
                                    numbered,
                                )
                            }
                            None => command_info_path(current_project.root_path(), &excluded_iris),
                        }
                    } else {
                        bail!(
                            "run outside of an active project, did you mean to use '--path' or '--iri'?"
                        )
                    }
                }
                (Location::Iri(iri), None) => crate::commands::info::command_info_uri(
                    iri,
                    !no_normalise,
                    client,
                    index_urls,
                    &excluded_iris,
                    runtime,
                ),
                (Location::Iri(iri), Some(subcommand)) => {
                    let numbered = subcommand.numbered();

                    crate::commands::info::command_info_verb_uri(
                        iri,
                        subcommand.as_verb(),
                        numbered,
                        client,
                        index_urls,
                        runtime,
                    )
                }
                (Location::Path(path), None) => command_info_path(Path::new(&path), &excluded_iris),
                (Location::Path(path), Some(subcommand)) => {
                    let numbered = subcommand.numbered();

                    command_info_verb_path(Path::new(&path), subcommand.as_verb(), numbered)
                }
            }
        }
        cli::Command::Add {
            iri,
            versions_constraint,
            no_lock,
            no_sync,
            dependency_opts,
        } => command_add(
            iri,
            versions_constraint,
            no_lock,
            no_sync,
            dependency_opts,
            &config,
            current_project,
            client,
            runtime,
        ),
        cli::Command::Remove { iri } => command_remove(iri, current_project),
        cli::Command::Include {
            paths,
            compute_checksum: add_checksum,
            no_index_symbols,
        } => command_include(paths, add_checksum, !no_index_symbols, current_project),
        cli::Command::Exclude { paths } => command_exclude(paths, current_project),
        cli::Command::Build { path } => {
            if let Some(current_project) = current_project {
                // Even if we are in a workspace, the project takes precedence.
                let path = if let Some(path) = path {
                    path
                } else {
                    let output_dir = current_workspace
                        .as_ref()
                        .map(|workspace| &workspace.workspace_path)
                        .unwrap_or_else(|| &current_project.project_path)
                        .join("output");
                    let name = sysand_core::build::default_kpar_file_name(&current_project)?;
                    if !output_dir.is_dir() {
                        std::fs::create_dir(&output_dir)?;
                    }
                    output_dir.join(name)
                };
                command_build_for_project(path, current_project)
            } else {
                // If the workspace is also missing, report an error about
                // missing project because that is what the user is more likely
                // to be looking for.
                let current_workspace =
                    current_workspace.ok_or(CliError::MissingProjectCurrentDir)?;
                let output_dir =
                    path.unwrap_or_else(|| current_workspace.workspace_path.join("output"));
                if !output_dir.is_dir() {
                    std::fs::create_dir(&output_dir)?;
                }
                command_build_for_workspace(output_dir, current_workspace)
            }
        }
        cli::Command::Sources { sources_opts } => {
            let cli::SourcesOptions {
                no_deps,
                include_std,
            } = sources_opts;
            let provided_iris = if !include_std {
                crate::logger::warn_std_omit();
                known_std_libs()
            } else {
                HashMap::default()
            };

            command_sources_project(
                !no_deps,
                current_project,
                current_environment,
                &provided_iris,
            )
        }
    }
}

pub fn get_env(project_root: &Path) -> Option<LocalDirectoryEnvironment> {
    let environment_path = project_root.join(DEFAULT_ENV_NAME);
    if !environment_path.is_dir() {
        None
    } else {
        Some(LocalDirectoryEnvironment { environment_path })
    }
}

pub fn get_or_create_env(project_root: &Path) -> Result<LocalDirectoryEnvironment> {
    match get_env(project_root) {
        Some(env) => Ok(env),
        None => command_env(project_root.join(DEFAULT_ENV_NAME)),
    }
}

fn get_config_verbose_quiet(config: &Config) -> (bool, bool) {
    (
        config.verbose.unwrap_or_default(),
        config.quiet.unwrap_or_default(),
    )
}

fn get_log_level(verbose: bool, quiet: bool) -> Result<log::LevelFilter> {
    match (verbose, quiet) {
        (true, true) => unreachable!(),
        (true, false) => Ok(log::LevelFilter::Debug),
        (false, true) => Ok(log::LevelFilter::Error),
        (false, false) => Ok(log::LevelFilter::Info),
    }
}
