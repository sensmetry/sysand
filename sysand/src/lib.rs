// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(not(feature = "std"))]
compile_error!("`std` feature is currently required to build `sysand`");

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    panic,
    process::ExitCode,
    str::FromStr,
    sync::Arc,
};

use anstream::{eprint, eprintln};
use anyhow::{Result, bail};
use fluent_uri::Iri;

use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;

use sysand_core::{
    auth::{HTTPAuthentication, StandardHTTPAuthenticationBuilder},
    config::{
        Config,
        local_fs::{get_config, load_configs},
    },
    env::local_directory::{DEFAULT_ENV_NAME, LocalDirectoryEnvironment},
    init::InitError,
    lock::Lock,
    project::{
        any::{AnyProject, OverrideProject},
        reference::ProjectReference,
        utils::wrapfs,
    },
    stdlib::known_std_libs,
};

use crate::{
    cli::{Args, InfoCommand},
    commands::{
        add::command_add,
        build::{command_build_for_project, command_build_for_workspace},
        env::{
            command_env, command_env_install, command_env_install_path, command_env_list,
            command_env_uninstall,
        },
        exclude::command_exclude,
        include::command_include,
        info::{command_info_current_project, command_info_path, command_info_verb_path},
        init::command_init,
        lock::command_lock,
        print_root::command_print_root,
        remove::command_remove,
        sources::{command_sources_env, command_sources_project},
        sync::command_sync,
    },
};

pub const DEFAULT_INDEX_URL: &str = "https://beta.sysand.org";

pub mod cli;
pub mod commands;
pub mod env_vars;
pub mod logger;
pub mod style;

mod error;
pub use error::CliError;

pub fn lib_main<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    set_panic_hook();

    match Args::try_parse_from(args) {
        Ok(args) => {
            if let Err(err) = run_cli(args) {
                let style = style::ERROR;
                eprint!("{style}error{style:#}: ");
                for cause in err.chain() {
                    eprintln!("{}", cause);
                }
                return ExitCode::FAILURE;
            }
        }
        Err(err) => {
            err.print().expect("failed to write Clap error");
            return ExitCode::from(err.exit_code() as u8);
        }
    }
    ExitCode::SUCCESS
}

fn set_panic_hook() {
    // TODO: use `panic::update_hook()` once it's stable
    //       also set bactrace style once it's stable, but take
    //       into account the current level
    let default_hook = panic::take_hook();
    // panic::set_backtrace_style(panic::BacktraceStyle::Short);
    panic::set_hook(Box::new(move |panic_info| {
        std::eprintln!(
            "Sysand crashed. This is a bug. We would appreciate a bug report at either\n\
            Sysand's issue tracker: https://github.com/sensmetry/sysand/issues\n\
            or Sensmetry forum: https://forum.sensmetry.com/c/sysand/24\n\
            or via email: sysand@sensmetry.com\n\
            \n\
            Below are details of the crash. It would be helpful to include them in the bug report."
        );
        default_hook(panic_info);
    }));
}

pub fn run_cli(args: cli::Args) -> Result<()> {
    sysand_core::style::set_style_config(crate::style::CONFIG);

    let log_level = get_log_level(args.global_opts.verbose, args.global_opts.quiet);
    if logger::init(log_level).is_err() {
        let warn = style::WARN;
        eprintln!(
            "{warn}warning{warn:#}: failed to set up logger because it has already been set up;\n\
            {:>8} log messages may not be formatted properly",
            ' '
        );
        log::set_max_level(log_level);
    }

    let current_workspace = sysand_core::discover::current_workspace()?;
    let current_project = sysand_core::discover::current_project()?;
    let cwd = wrapfs::current_dir()?;

    let project_root = current_project.as_ref().map(|p| p.root_path().to_owned());

    let current_environment = {
        let dir = project_root.as_ref().unwrap_or(&cwd);
        crate::get_env(dir)?
    };

    let auto_config = if args.global_opts.no_config {
        Config::default()
    } else {
        load_configs(project_root.as_deref().unwrap_or(Utf8Path::new(".")))?
    };

    let mut config = if let Some(config_file) = &args.global_opts.config_file {
        get_config(config_file)?
    } else {
        Config::default()
    };

    config.merge(auto_config);

    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap(),
    );

    let _runtime_keepalive = runtime.clone();

    // FIXME: This is a temporary implementation to provide credentials until
    //        https://github.com/sensmetry/sysand/pull/157
    //        gets merged.
    let mut auth_patterns = HashMap::new();
    let mut basic_auth_users = HashMap::new();
    let mut basic_auth_passwords = HashMap::new();
    let mut bearer_auth_tokens = HashMap::new();

    for (key, value) in std::env::vars() {
        if let Some(key_rest) = key.strip_prefix("SYSAND_CRED_") {
            if let Some(key_name) = key_rest.strip_suffix("_BASIC_USER") {
                basic_auth_users.insert(key_name.to_owned(), value);
            } else if let Some(key_name) = key_rest.strip_suffix("_BASIC_PASS") {
                basic_auth_passwords.insert(key_name.to_owned(), value);
            } else if let Some(key_name) = key_rest.strip_suffix("_BEARER_TOKEN") {
                bearer_auth_tokens.insert(key_name.to_owned(), value);
            } else {
                auth_patterns.insert(key_rest.to_owned(), value);
            }
        }
    }

    let mut basic_auth_pattern_names = HashSet::new();
    for x in [
        &auth_patterns,
        &basic_auth_users,
        &basic_auth_passwords,
        &bearer_auth_tokens,
    ] {
        for k in x.keys() {
            basic_auth_pattern_names.insert(k);
        }
    }

    let mut auths_builder: StandardHTTPAuthenticationBuilder =
        StandardHTTPAuthenticationBuilder::new();
    for k in basic_auth_pattern_names {
        match (
            auth_patterns.get(k),
            basic_auth_users.get(k),
            basic_auth_passwords.get(k),
            bearer_auth_tokens.get(k),
        ) {
            (Some(_), None, None, None) => {
                anyhow::bail!(
                    "SYSAND_CRED_{k} has no matching authentication scheme, please specify SYSAND_CRED_{k}_BASIC_USER/SYSAND_CRED_{k}_BASIC_PASS or SYSAND_CRED_{k}_BEARER_TOKEN"
                );
            }
            (Some(pattern), maybe_username, maybe_password, maybe_token) => {
                let mut matched_schemes = 0;

                match (maybe_username, maybe_password) {
                    (Some(username), Some(password)) => {
                        matched_schemes += 1;
                        auths_builder.add_basic_auth(pattern, username, password)
                    }
                    (None, None) => {}
                    (_, _) => {
                        anyhow::bail!(
                            "Please specify both (or neither) of SYSAND_CRED_{k}_BASIC_USER and SYSAND_CRED_{k}_BASIC_PASS"
                        );
                    }
                }

                if let Some(token) = maybe_token {
                    matched_schemes += 1;
                    auths_builder.add_bearer_auth(pattern, token);
                }

                if matched_schemes > 1 {
                    log::warn!("SYSAND_CRED_{k} has multiple authentication schemes!");
                }
            }
            (None, _, _, _) => {
                anyhow::bail!("please specify URL pattern SYSAND_CRED_{k} for credential");
            }
        }
    }
    let basic_auth_policy = Arc::new(auths_builder.build()?);

    match args.command {
        cli::Command::Init {
            path,
            name,
            version,
            no_semver,
            license,
            no_spdx,
        } => command_init(name, version, no_semver, license, no_spdx, path),
        cli::Command::Env { command } => match command {
            None => {
                let env_dir = {
                    let mut p = project_root.unwrap_or(cwd);
                    p.push(DEFAULT_ENV_NAME);
                    p
                };
                command_env(env_dir)?;

                Ok(())
            }
            Some(cli::EnvCommand::Install {
                iri,
                version,
                path,
                install_opts,
                resolution_opts,
            }) => {
                if let Some(path) = path {
                    command_env_install_path(
                        iri,
                        version,
                        path,
                        install_opts,
                        resolution_opts,
                        &config,
                        project_root,
                        client,
                        runtime,
                        basic_auth_policy,
                    )
                } else {
                    command_env_install(
                        iri,
                        version,
                        install_opts,
                        resolution_opts,
                        &config,
                        project_root,
                        client,
                        runtime,
                        basic_auth_policy,
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
        cli::Command::Lock { resolution_opts } => {
            if let Some(project_root) = project_root {
                crate::commands::lock::command_lock(
                    ".",
                    resolution_opts,
                    &config,
                    project_root,
                    client,
                    runtime,
                    basic_auth_policy,
                )
                .map(|_| ())
            } else {
                bail!("not inside a project")
            }
        }
        cli::Command::Sync { resolution_opts } => {
            let cli::ResolutionOptions { include_std, .. } = resolution_opts.clone();
            let mut local_environment = match current_environment {
                Some(env) => env,
                None => command_env(project_root.as_ref().unwrap_or(&cwd).join(DEFAULT_ENV_NAME))?,
            };

            let provided_iris = if !include_std {
                crate::logger::warn_std_deps();
                known_std_libs()
            } else {
                HashMap::default()
            };
            let project_root = project_root.unwrap_or(cwd);
            let lockfile = project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);
            if !wrapfs::is_file(&lockfile)? {
                command_lock(
                    ".",
                    resolution_opts,
                    &config,
                    project_root.clone(),
                    client.clone(),
                    runtime.clone(),
                    basic_auth_policy.clone(),
                )?;
            }
            let lock = Lock::from_str(&wrapfs::read_to_string(lockfile)?)?;
            command_sync(
                &lock,
                project_root,
                &mut local_environment,
                client,
                &provided_iris,
                runtime,
                basic_auth_policy,
            )
        }
        cli::Command::PrintRoot => command_print_root(cwd),
        cli::Command::Info {
            path,
            iri,
            auto_location,
            no_normalise,
            resolution_opts,
            subcommand,
        } => {
            let cli::ResolutionOptions {
                index,
                default_index,
                no_index,
                include_std,
            } = resolution_opts;
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
                // Only print std warning when command is to print all info
                // or just usages.
                // These are the only cases where stdlib usages affect output
                match subcommand {
                    None
                    | Some(InfoCommand::Usage {
                        clear: None,
                        add: None,
                        set: None,
                        remove: None,
                        numbered: _,
                    }) => crate::logger::warn_std_deps(),
                    _ => (),
                }
                known_std_libs().keys().cloned().collect()
            } else {
                HashSet::default()
            };

            let project_root = project_root.unwrap_or(wrapfs::current_dir()?);
            let overrides = get_overrides(
                &config,
                &project_root,
                &client,
                runtime.clone(),
                basic_auth_policy.clone(),
            )?;

            enum Location {
                WorkDir,
                Iri(fluent_uri::Iri<String>),
                Path(Utf8PathBuf),
            }

            let location = if let Some(auto_location) = auto_location {
                debug_assert!(path.is_none());
                debug_assert!(iri.is_none());

                if let Ok(iri) = fluent_uri::Iri::parse(auto_location.clone()) {
                    Location::Iri(iri)
                } else {
                    Location::Path(auto_location.into())
                }
            } else if let Some(path) = path {
                debug_assert!(auto_location.is_none());
                debug_assert!(iri.is_none());

                Location::Path(path.into())
            } else if let Some(iri) = iri {
                debug_assert!(path.is_none());
                debug_assert!(auto_location.is_none());

                Location::Iri(iri)
            } else {
                Location::WorkDir
            };

            match (location, subcommand) {
                (Location::WorkDir, subcommand) => {
                    if let Some(current_project) = sysand_core::discover::current_project()? {
                        match subcommand {
                            Some(subcommand) => {
                                match subcommand {
                                    cli::InfoCommand::Version {
                                        ref set, no_semver, ..
                                    } => {
                                        // TODO(MSRV 1.88):
                                        // if let Some(v) = set
                                        //     && !no_semver
                                        if !no_semver {
                                            if let Some(v) = set {
                                                semver::Version::parse(v).map_err(|e| {
                                                InitError::<std::convert::Infallible>::SemVerParse(
                                                    v.as_str().into(),
                                                    e,
                                                )
                                            })?;
                                            }
                                        }
                                    }
                                    cli::InfoCommand::License {
                                        ref set, no_spdx, ..
                                    } => {
                                        // TODO(MSRV 1.88):
                                        // if let Some(v) = set
                                        //     && !no_spdx
                                        if !no_spdx {
                                            if let Some(l) = set {
                                                spdx::Expression::parse(l).map_err(|e| {
                                                InitError::<std::convert::Infallible>::SPDXLicenseParse(l.as_str().into(), e)
                                            })?;
                                            }
                                        }
                                    }
                                    _ => (),
                                }

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
                            "run outside of an active project, did you mean to use `--path` or `--iri`?"
                        )
                    }
                }
                (Location::Iri(iri), None) => crate::commands::info::command_info_uri(
                    iri,
                    !no_normalise,
                    client,
                    index_urls,
                    &excluded_iris,
                    overrides,
                    runtime,
                    basic_auth_policy,
                ),
                (Location::Iri(iri), Some(subcommand)) => {
                    let numbered = subcommand.numbered();

                    crate::commands::info::command_info_verb_uri(
                        iri,
                        subcommand.as_verb(),
                        numbered,
                        client,
                        index_urls,
                        overrides,
                        runtime,
                        basic_auth_policy,
                    )
                }
                (Location::Path(path), None) => command_info_path(&path, &excluded_iris),
                (Location::Path(path), Some(subcommand)) => {
                    let numbered = subcommand.numbered();

                    command_info_verb_path(&path, subcommand.as_verb(), numbered)
                }
            }
        }
        cli::Command::Add {
            iri,
            version_constraint,
            no_lock,
            no_sync,
            resolution_opts,
            source_opts,
        } => command_add(
            iri,
            version_constraint,
            no_lock,
            no_sync,
            resolution_opts,
            source_opts,
            config,
            args.global_opts.config_file,
            args.global_opts.no_config,
            current_project,
            client,
            runtime,
            basic_auth_policy,
        ),
        cli::Command::Remove { iri } => command_remove(
            iri,
            current_project,
            args.global_opts.config_file,
            args.global_opts.no_config,
        ),
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
                    let mut output_dir = current_workspace
                        .as_ref()
                        .map(|workspace| workspace.root_path())
                        .unwrap_or_else(|| &current_project.project_path)
                        .join("output");
                    let name = sysand_core::build::default_kpar_file_name(&current_project)?;
                    if !wrapfs::is_dir(&output_dir)? {
                        wrapfs::create_dir(&output_dir)?;
                    }
                    output_dir.push(name);
                    output_dir
                };
                command_build_for_project(path, current_project)
            } else {
                // If the workspace is also missing, report an error about
                // missing project because that is what the user is more likely
                // to be looking for.
                let current_workspace =
                    current_workspace.ok_or(CliError::MissingProjectCurrentDir)?;
                let output_dir =
                    path.unwrap_or_else(|| current_workspace.root_path().join("output"));
                if !wrapfs::is_dir(&output_dir)? {
                    wrapfs::create_dir(&output_dir)?;
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
        cli::Command::Clone {
            locator,
            version,
            target,
            resolution_opts,
            no_deps,
        } => commands::clone::command_clone(
            locator,
            version,
            target,
            no_deps,
            resolution_opts,
            &config,
            client,
            runtime,
            basic_auth_policy,
        ),
    }
}

pub fn get_env(project_root: impl AsRef<Utf8Path>) -> Result<Option<LocalDirectoryEnvironment>> {
    let environment_path = project_root.as_ref().join(DEFAULT_ENV_NAME);
    let env = wrapfs::is_dir(&environment_path)?
        .then_some(LocalDirectoryEnvironment { environment_path });
    Ok(env)
}

pub fn get_or_create_env(project_root: impl AsRef<Utf8Path>) -> Result<LocalDirectoryEnvironment> {
    let project_root = project_root.as_ref();
    match get_env(project_root)? {
        Some(env) => Ok(env),
        None => command_env(project_root.join(DEFAULT_ENV_NAME)),
    }
}

fn get_log_level(verbose: bool, quiet: bool) -> log::LevelFilter {
    match (verbose, quiet) {
        (true, true) => unreachable!(),
        (true, false) => log::LevelFilter::Debug,
        (false, true) => log::LevelFilter::Error,
        (false, false) => log::LevelFilter::Info,
    }
}

pub type Overrides<Policy> = Vec<(Iri<String>, Vec<OverrideProject<Policy>>)>;

pub fn get_overrides<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    config: &Config,
    project_root: P,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<Overrides<Policy>> {
    let mut overrides = Vec::new();
    for config_project in &config.projects {
        for identifier in &config_project.identifiers {
            let mut projects = Vec::new();
            for source in &config_project.sources {
                projects.push(ProjectReference::new(AnyProject::try_from_source(
                    source.clone(),
                    &project_root,
                    auth_policy.clone(),
                    client.clone(),
                    runtime.clone(),
                )?));
            }
            overrides.push((Iri::parse(identifier.as_str())?.into(), projects));
        }
    }
    Ok(overrides)
}
