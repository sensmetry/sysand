// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(not(feature = "std"))]
compile_error!("`std` feature is currently required to build `sysand`");

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::ErrorKind,
    process::ExitCode,
    str::FromStr as _,
    sync::Arc,
};

use anstream::eprintln;
use anyhow::{Result, anyhow, bail};
use fluent_uri::Iri;

use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use sysand_core::{
    auth::{HTTPAuthentication, StandardHTTPAuthenticationBuilder, StandardLazyHTTPAuthentication},
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{
        Config,
        local_fs::{get_config, load_configs},
    },
    context::ProjectContext,
    discover::{discover_project, discover_workspace},
    env::{DEFAULT_ENV_NAME, local_directory::LocalDirectoryEnvironment},
    index::RemoveTarget,
    init::InitError,
    lock::Lock,
    project::{
        any::{AnyProject, OverrideProject},
        local_src::LocalSrcProject,
        reference::ProjectReference,
        utils::{Identifier, wrapfs},
    },
    resolve::net_utils::create_reqwest_client,
    stdlib::known_std_libs,
    utils::format_err,
    workspace::Workspace,
};
use url::Url;

use crate::{
    cli::{Args, AuthCommand, Command, EnvCommand, ExpCommand, IndexCommand, InfoCommand},
    commands::{
        add::{ExpAddArgs, command_add, exp_command_add},
        auth::{command_auth_login, command_auth_logout, command_auth_status, command_auth_whoami},
        build::{command_build_for_project, command_build_for_workspace},
        clone::command_clone,
        env::{
            command_env, command_env_install, command_env_install_path, command_env_list,
            command_env_uninstall,
        },
        exclude::command_exclude,
        include::command_include,
        index::{command_index_add, command_index_init, command_index_remove, command_index_yank},
        info::{command_info_current_project, command_info_path, command_info_verb_path},
        init::command_init,
        lock::command_lock,
        print_root::command_print_root,
        publish::command_publish,
        remove::{command_remove, exp_command_remove},
        sources::{command_sources_env, command_sources_project},
        sync::command_sync,
    },
};

pub const DEFAULT_INDEX_URL: &str = "https://sysand.com";

/// The CLI's composed authentication policy: eager `SYSAND_CRED_*`
/// credentials first, then lazily read stored credentials from the OS
/// keyring.
pub type CliAuthPolicy = StandardLazyHTTPAuthentication<credential_store::CliBlobBackend>;

pub mod cli;
pub mod commands;
pub(crate) mod cred_env;
pub mod credential_store;
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
    #[cfg(not(debug_assertions))]
    set_panic_hook();

    match Args::try_parse_from(args) {
        Ok(args) => {
            if let Err(err) = run_cli(args) {
                let style = style::ERROR;
                eprintln!("{style}error{style:#}: {err}");
                let mut causes = err.chain();
                // The first cause is the error itself which is printed already
                _ = causes.next();
                for cause in causes {
                    eprintln!("{style}  caused by:{style:#} {cause}");
                }
                let note_style = style::GOOD;
                if log::max_level() < log::Level::Debug {
                    eprintln!(
                        "\n{note_style}note{note_style:#}: pass `-v`/`--verbose` to output additional logs"
                    );
                }
                return ExitCode::FAILURE;
            }
        }
        Err(err) => {
            err.print().expect("failed to write Clap error");
            // `exit_code()` is non-negative and within u8
            #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            return ExitCode::from(err.exit_code() as u8);
        }
    }
    ExitCode::SUCCESS
}

// Clutters panic output, so disabled in debug builds
#[cfg(not(debug_assertions))]
fn set_panic_hook() {
    use std::panic;
    // TODO: use `panic::update_hook()` once it's stable
    //       also set backtrace style once it's stable, but take
    //       into account the current level
    let default_hook = panic::take_hook();
    // panic::set_backtrace_style(panic::BacktraceStyle::Short);
    panic::set_hook(Box::new(move |panic_info| {
        std::eprintln!(
            "\n\n\
            Sysand crashed. This is likely a bug. We would appreciate a bug report at either\n\
            Sysand issue tracker: https://github.com/sensmetry/sysand/issues\n\
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

    let cwd = wrapfs::current_dir()?;
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
    log::debug!("sysand v{}", env!("CARGO_PKG_VERSION"));

    let current_project = discover_project(&cwd)?;
    let current_workspace = discover_workspace(&cwd)?;
    let env = match (&current_workspace, &current_project) {
        // TODO: does it make sense to support env unassociated with a project
        // when index and env are different?
        (None, None) => get_env(&cwd)?,
        (None, Some(pr)) => get_env(pr.root_path())?,
        (Some(w), _) => get_env(w.root_path())?,
    };
    let ctx = ProjectContext {
        env,
        current_workspace,
        current_project,
        current_directory: cwd,
    };
    let project_root = ctx
        .current_project
        .as_ref()
        .map(|p| p.root_path().to_owned());

    let auto_config = if args.global_opts.no_config {
        Config::default()
    } else {
        #[expect(clippy::or_fun_call, reason = "cheap")]
        load_configs(project_root.as_deref().unwrap_or(Utf8Path::new(".")))?
    };

    let mut config = if let Some(config_file) = &args.global_opts.config_file {
        get_config(config_file)?
    } else {
        Config::default()
    };

    config.merge(auto_config);

    let client = create_reqwest_client()?;

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap(),
    );

    let _runtime_keep_alive = runtime.clone();

    // The `auth` commands manage credentials rather than use them, so
    // they dispatch before the eager env-policy build below. `status`
    // applies the same strict `SYSAND_CRED_*` validation on its own;
    // `login`, `logout`, and `whoami` must keep working even when those
    // variables are malformed. `login` still gets the shared client and
    // runtime: core
    // handles its discovery fetch itself (an unauthenticated baseline
    // with a forced-bearer retry carrying the just-entered secret), so
    // no ambient auth policy is needed.
    let command = match args.command {
        Command::Auth { command } => {
            return match command {
                AuthCommand::Status => command_auth_status(&config),
                AuthCommand::Login {
                    index_url,
                    token_stdin,
                } => command_auth_login(index_url, token_stdin, &config, &client, runtime),
                AuthCommand::Whoami { index_url } => {
                    command_auth_whoami(index_url, &config, &client, &runtime)
                }
                AuthCommand::Logout { index_url } => command_auth_logout(index_url, &config),
            };
        }
        command => command,
    };

    // Validation guarantees every group has a pattern and at least one
    // complete scheme, so iterating the patterns covers every group.
    let groups = cred_env::validated_env_groups()?;
    let mut auths_builder = StandardHTTPAuthenticationBuilder::new();
    for (k, pattern) in &groups.patterns {
        if let (Some(username), Some(password)) =
            (groups.basic_users.get(k), groups.basic_passwords.get(k))
        {
            log::debug!("auth: env vars specify HTTP basic for URL glob `{pattern}`");
            auths_builder.add_basic_auth(pattern, username, password);
        }
        if let Some(token) = groups.bearer_tokens.get(k) {
            log::debug!("auth: env vars specify bearer token for URL glob `{pattern}`");
            // The label lets a publish auth failure name the
            // `SYSAND_CRED_<LABEL>` variable to fix.
            auths_builder.add_bearer_auth(pattern, token, k);
        }
    }
    let env_auth_policy = auths_builder.build()?;
    // Compose the eager env policy with the lazily read OS keyring store.
    // Opening the store only resolves a lock file path; no keychain access
    // happens until a request actually needs a stored credential.
    let auth_policy = Arc::new(match credential_store::open_cli_credential_store() {
        Ok(store) => CliAuthPolicy::new(env_auth_policy, store),
        Err(err) => {
            log::warn!(
                "credential store unavailable: {err};\n\
                 continuing with `SYSAND_CRED_*` credentials only"
            );
            CliAuthPolicy::without_store(env_auth_policy)
        }
    });

    match command {
        Command::Init {
            path,
            name,
            publisher,
            version,
            no_semver,
            license,
            no_spdx,
        } => command_init(
            name, publisher, version, no_semver, license, no_spdx, path, ctx,
        ),
        Command::New { .. } => bail!("use `init` instead of `new`"),
        Command::Env { command } => match command {
            None => {
                let env_dir = {
                    let mut p = project_root.unwrap_or(ctx.current_directory);
                    p.push(DEFAULT_ENV_NAME);
                    p
                };
                command_env(env_dir)?;

                Ok(())
            }
            Some(EnvCommand::Install {
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
                        auth_policy,
                        ctx,
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
                        auth_policy,
                        ctx,
                    )
                }
            }
            Some(EnvCommand::Uninstall { iri, version }) => {
                if let Some(local_environment) = ctx.env {
                    command_env_uninstall(iri, version, local_environment)
                } else {
                    log::warn!("no environment to uninstall from");
                    Ok(())
                }
            }
            Some(EnvCommand::List) => command_env_list(ctx.env),
            Some(EnvCommand::Sources {
                iri,
                version,
                sources_opts,
            }) => command_sources_env(
                iri,
                version,
                sources_opts.only_deps,
                sources_opts.deps,
                ctx.env,
            ),
        },
        Command::Index { command } => {
            let root =
                |index_root: Option<Utf8PathBuf>| index_root.unwrap_or(ctx.current_directory);
            match command {
                IndexCommand::Init { index_root } => command_index_init(root(index_root))?,
                IndexCommand::Add {
                    iri,
                    kpar_path,
                    index_root,
                } => command_index_add(iri, kpar_path, root(index_root))?,
                IndexCommand::Yank {
                    iri,
                    version,
                    index_root,
                } => command_index_yank(iri, version, root(index_root))?,
                IndexCommand::Remove {
                    iri,
                    target,
                    index_root,
                } => {
                    let target = match (target.version, target.project) {
                        (Some(version), false) => RemoveTarget::Version(version),
                        (None, true) => RemoveTarget::Project,
                        _ => unreachable!(),
                    };
                    command_index_remove(iri, target, root(index_root))?
                }
            }
            Ok(())
        }
        Command::Lock { resolution_opts } => {
            if let Some(project_root) = project_root {
                command_lock(
                    ".",
                    resolution_opts,
                    &config,
                    project_root,
                    client,
                    runtime,
                    auth_policy,
                    &ctx,
                )
                .map(|_| ())
            } else {
                bail!(
                    "not inside a project - neither current nor any of the parent directories contain a SysML v2 or KerML project"
                )
            }
        }
        Command::Sync {
            resolution_opts,
            no_prune,
        } => {
            let provided_iris = if resolution_opts.include_std {
                HashMap::default()
            } else {
                known_std_libs()
            };

            let project_root = project_root.unwrap_or_else(|| ctx.current_directory.clone());
            let lockfile = project_root.join(DEFAULT_LOCKFILE_NAME);
            let lock = match fs::read_to_string(&lockfile) {
                Ok(l) => match Lock::from_str(&l) {
                    Ok(l) => l,
                    // Include file path in errors
                    Err(e) => bail!("invalid lockfile `{lockfile}`:\n{}", format_err(e)),
                },
                Err(e) => {
                    if e.kind() == ErrorKind::NotFound {
                        command_lock(
                            ".",
                            resolution_opts,
                            &config,
                            &project_root,
                            client.clone(),
                            runtime.clone(),
                            auth_policy.clone(),
                            &ctx,
                        )?
                    } else {
                        bail!("failed to read lockfile `{lockfile}`: {}", format_err(e))
                    }
                }
            };
            let mut local_environment = get_or_create_env(
                ctx.env,
                ctx.current_workspace.as_ref(),
                ctx.current_project.as_ref(),
                &ctx.current_directory,
            )?;
            command_sync(
                &lock,
                project_root,
                &mut local_environment,
                client,
                &provided_iris,
                runtime,
                auth_policy,
                ctx.current_workspace.as_ref(),
                no_prune,
            )
        }
        Command::Auth { .. } => {
            unreachable!("`auth` is dispatched before the auth policy is built")
        }
        Command::PrintRoot => command_print_root(ctx.current_directory),
        Command::Info {
            path,
            iri,
            auto_location,
            no_normalise,
            resolution_opts,
            subcommand,
        } => {
            enum Location {
                WorkDir,
                Iri(fluent_uri::Iri<String>),
                Path(Utf8PathBuf),
            }

            let cli::ResolutionOptions {
                index,
                default_index,
                no_index,
                include_std,
            } = resolution_opts;
            let index_urls = if no_index {
                None
            } else {
                Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_owned()], default_index)?)
            };
            let excluded_usages: HashSet<_> = if include_std {
                HashSet::default()
            } else {
                known_std_libs().into_keys().collect()
            };

            let project_root = project_root.as_ref().unwrap_or(&ctx.current_directory);
            let overrides = get_overrides(
                &config,
                project_root,
                &client,
                runtime.clone(),
                auth_policy.clone(),
            )?;

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

                Location::Path(path)
            } else if let Some(iri) = iri {
                debug_assert!(path.is_none());
                debug_assert!(auto_location.is_none());

                Location::Iri(iri)
            } else {
                Location::WorkDir
            };

            match (location, subcommand) {
                (Location::WorkDir, subcommand) => {
                    if let Some(current_project) = ctx.current_project {
                        match subcommand {
                            Some(subcommand) => {
                                match &subcommand {
                                    InfoCommand::Version { set, no_semver, .. } => {
                                        if !no_semver && let Some(v) = set {
                                            semver::Version::parse(v).map_err(|e| {
                                                InitError::<std::convert::Infallible>::SemVerParse(
                                                    v.as_str().into(),
                                                    e,
                                                )
                                            })?;
                                        }
                                    }
                                    InfoCommand::License { set, no_spdx, .. } => {
                                        if !no_spdx && let Some(l) = set {
                                            spdx::Expression::parse(l).map_err(|e| {
                                                InitError::<std::convert::Infallible>::SPDXLicenseParse(l.as_str().into(), e)
                                            })?;
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
                            None => {
                                command_info_path(current_project.root_path(), &excluded_usages)
                            }
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
                    &excluded_usages,
                    overrides,
                    runtime,
                    auth_policy,
                    ctx,
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
                        auth_policy,
                        ctx,
                    )
                }
                (Location::Path(path), None) => command_info_path(&path, &excluded_usages),
                (Location::Path(path), Some(subcommand)) => {
                    let numbered = subcommand.numbered();

                    command_info_verb_path(&path, subcommand.as_verb(), numbered)
                }
            }
        }
        Command::Add {
            locator,
            version_constraint,
            no_lock,
            no_sync,
            resolution_opts,
            source_opts,
            no_prune,
        } => {
            let iri = iri_or_path_to_iri(locator.iri, locator.path)?;
            command_add(
                iri,
                version_constraint,
                no_lock,
                no_sync,
                no_prune,
                resolution_opts,
                source_opts,
                config,
                args.global_opts.config_file,
                args.global_opts.no_config,
                ctx,
                client,
                runtime,
                auth_policy,
            )
        }
        Command::Remove {
            locator,
            no_lock,
            no_sync,
            no_prune,
            resolution_opts,
        } => {
            let iri = iri_or_path_to_iri(locator.iri, locator.path)?;
            command_remove(
                iri,
                ctx,
                config,
                args.global_opts.config_file,
                args.global_opts.no_config,
                no_lock,
                no_sync,
                no_prune,
                resolution_opts,
                client,
                runtime,
                auth_policy,
            )
        }
        Command::Include {
            paths,
            compute_checksum: add_checksum,
            no_index_symbols,
        } => command_include(paths, add_checksum, !no_index_symbols, ctx),
        Command::Exclude { paths } => command_exclude(paths, ctx),
        Command::Build {
            path,
            compression,
            allow_path_usage,
            keep_index,
        } => {
            if let Some(current_project) = ctx.current_project {
                // Even if we are in a workspace, the project takes precedence.
                let path = if let Some(path) = path {
                    path
                } else {
                    let path = sysand_core::build::default_kpar_path(
                        &current_project,
                        ctx.current_workspace.as_ref(),
                        current_project.root_path(),
                    )?;
                    if let Some(output_dir) = path.parent()
                        && !wrapfs::is_dir(output_dir)?
                    {
                        wrapfs::create_dir(output_dir)?;
                    }
                    path
                };
                command_build_for_project(
                    path,
                    compression.into(),
                    current_project,
                    !keep_index,
                    allow_path_usage,
                )
            } else {
                // If the workspace is also missing, report an error about
                // missing project because that is what the user is more likely
                // to be looking for.
                let current_workspace = ctx
                    .current_workspace
                    .ok_or(CliError::MissingProjectCurrentDir)?;
                let output_dir =
                    path.unwrap_or_else(|| current_workspace.root_path().join("output"));
                if !wrapfs::is_dir(&output_dir)? {
                    wrapfs::create_dir(&output_dir)?;
                }
                command_build_for_workspace(
                    output_dir,
                    compression.into(),
                    current_workspace,
                    !keep_index,
                    allow_path_usage,
                )
            }
        }
        Command::Publish {
            path,
            index,
            trusted_publishing,
        } => command_publish(
            path,
            index,
            trusted_publishing,
            &ctx,
            auth_policy,
            client,
            runtime,
        ),
        Command::Sources { sources_opts } => {
            command_sources_project(sources_opts.only_deps, sources_opts.deps, ctx)
        }
        Command::Clone {
            locator,
            version,
            target,
            resolution_opts,
            no_deps,
        } => command_clone(
            locator,
            version,
            target,
            ctx,
            no_deps,
            resolution_opts,
            &config,
            client,
            runtime,
            auth_policy,
        ),
        Command::Experimental { subcommand } => match subcommand {
            ExpCommand::Add {
                locator,
                resolution_opts,
                no_lock,
                no_sync,
                no_prune,
            } => {
                let add = if let Some(dir) = locator.dir {
                    ExpAddArgs::Dir { dir }
                } else if let Some(kpar_path) = locator.kpar_path {
                    ExpAddArgs::KparPath { kpar_path }
                } else {
                    unreachable!("clap group requires exactly one of `dir`/`kpar_path`")
                };
                exp_command_add(
                    add,
                    no_lock,
                    no_sync,
                    no_prune,
                    resolution_opts,
                    config,
                    ctx,
                    client,
                    runtime,
                    auth_policy,
                )
            }
            ExpCommand::Remove {
                publisher,
                name,
                no_lock,
                no_sync,
                no_prune,
                resolution_opts,
            } => exp_command_remove(
                publisher,
                name,
                ctx,
                config,
                no_lock,
                no_sync,
                no_prune,
                resolution_opts,
                client,
                runtime,
                auth_policy,
            ),
        },
    }
}

fn iri_or_path_to_iri(
    iri: Option<Iri<String>>,
    path: Option<Utf8PathBuf>,
) -> Result<Iri<String>, anyhow::Error> {
    Ok(if let Some(iri) = iri {
        iri
    } else {
        let Some(path) = path else { unreachable!() };
        let abs_path = wrapfs::canonicalize(&path)?;
        let url: String = Url::from_file_path(abs_path)
            .map_err(|()| anyhow!("unsupported path type of `{path}`"))?
            .into();
        Iri::parse(url).expect("BUG: file URL from path is invalid IRI")
    })
}

/// Read `root/.sysand/` metadata
pub fn get_env(root: impl AsRef<Utf8Path>) -> Result<Option<LocalDirectoryEnvironment>> {
    let environment_path = root.as_ref().join(DEFAULT_ENV_NAME);
    LocalDirectoryEnvironment::try_read(environment_path).map_err(anyhow::Error::from)
}

/// Unpack `env`, or create an empty environment otherwise
pub fn get_or_create_env(
    env: Option<LocalDirectoryEnvironment>,
    workspace: Option<&Workspace>,
    project: Option<&LocalSrcProject>,
    cwd: impl AsRef<Utf8Path>,
) -> Result<LocalDirectoryEnvironment> {
    if let Some(env) = env {
        return Ok(env);
    }
    let base_path = match (workspace, project) {
        (None, None) => cwd.as_ref(),
        (None, Some(pr)) => pr.root_path(),
        (Some(w), _) => w.root_path(),
    };
    command_env(base_path.join(DEFAULT_ENV_NAME))
}

fn get_log_level(verbose: bool, quiet: bool) -> log::LevelFilter {
    match (verbose, quiet) {
        (true, true) => unreachable!(),
        (true, false) => log::LevelFilter::Debug,
        (false, true) => log::LevelFilter::Error,
        (false, false) => log::LevelFilter::Info,
    }
}

pub type Overrides<Policy> = Vec<(Identifier, Vec<OverrideProject<Policy>>)>;

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
                projects.push(ProjectReference::new(AnyProject::try_from_override_source(
                    source.clone(),
                    &project_root,
                    auth_policy.clone(),
                    client.clone(),
                    runtime.clone(),
                )?));
            }
            overrides.push((
                Identifier::from_iri(&Iri::parse(identifier.as_str())?),
                projects,
            ));
        }
    }
    Ok(overrides)
}

/// Quote a string for a POSIX shell or CMD/PowerShell. CMD and
/// PowerShell have slightly different semantics, so this uses their
/// common behaviour only.
///
/// Likely to handle edge cases incorrectly, so only suitable for suggestions
pub fn quote_for_shell(arg: &str) -> String {
    #[cfg(unix)]
    {
        // original length + 2 surrounding quotes
        let mut out = String::with_capacity(arg.len() + 2);
        out.push('\'');

        for c in arg.chars() {
            if c == '\'' {
                // `\` is not allowed inside '' string, so end the string,
                // put the escaped `'` and start another string. Strings
                // without a space between them will be concatenated into a
                // single arg by the shell
                out.push_str(r"'\''");
            } else {
                out.push(c);
            }
        }

        out.push('\'');
        out
    }

    #[cfg(windows)]
    {
        use std::iter::repeat_n;

        let mut out = String::with_capacity(arg.len() + 2);
        out.push('"');

        let mut backslashes = 0;

        for c in arg.chars() {
            // Special `\` handling to work with CommandLineToArgvW
            // https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-commandlinetoargvw#remarks
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    out.extend(repeat_n('\\', backslashes * 2 + 1));
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    out.extend(repeat_n('\\', backslashes));
                    backslashes = 0;
                    out.push(c);
                }
            }
        }

        out.extend(repeat_n('\\', backslashes * 2));
        out.push('"');
        out
    }
}

#[cfg(test)]
#[path = "./lib_tests.rs"]
mod tests;
