// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, fs, io::ErrorKind, str::FromStr as _, sync::Arc};

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use reqwest_middleware::ClientWithMiddleware;
use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{
        Config,
        local_fs::{CONFIG_FILE, remove_project_source_from_config},
    },
    context::ProjectContext,
    lock::Lock,
    model::InterchangeProjectUsageRaw,
    project::{
        ProjectRead as _,
        utils::{Identifier, wrapfs},
    },
    remove::{do_remove, exp_do_remove},
    utils::format_err,
};

use crate::{
    CliError,
    cli::ResolutionOptions,
    commands::{add::resolve_deps, sync::command_sync},
};

#[expect(clippy::fn_params_excessive_bools)]
pub fn command_remove<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    mut ctx: ProjectContext,
    config: Config,
    config_file: Option<String>,
    no_config: bool,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let current_project = ctx
        .current_project
        .as_mut()
        .ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path().to_owned();
    let config_path = config_file
        .map(Utf8PathBuf::from)
        .or_else(|| (!no_config).then(|| current_project.root_path().join(CONFIG_FILE)));

    let iri_copy = iri.to_string();
    // `.project.json` is not backed up here, since the failure to lock
    // or sync cannot logically be caused by the remove command itself
    // (unlike `add`), the failure must be pre-existing or transient
    // (i.e. lock/sync would have also failed even without the `remove`).
    // Therefore lock/sync failures should not revert the removal
    let usages = do_remove(current_project, iri.into_string())?;
    print_removed(&usages);

    if !no_lock {
        lock_sync(
            ctx,
            config,
            no_sync,
            no_prune,
            resolution_opts,
            client,
            runtime,
            auth_policy,
            project_root,
            &iri_copy,
        )?;
    }

    // This has to be done after the resolution
    // TODO: this is not always correct, as config file overrides also
    // affect transitive dependencies
    if let Some(path) = config_path {
        remove_project_source_from_config(path, &iri_copy)?;
    }

    Ok(())
}

pub fn exp_command_remove<Policy: HTTPAuthentication>(
    publisher: impl AsRef<str>,
    name: impl AsRef<str>,
    mut ctx: ProjectContext,
    config: Config,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let publisher = publisher.as_ref();
    let name = name.as_ref();
    let current_project = ctx
        .current_project
        .as_mut()
        .ok_or(CliError::MissingProjectCurrentDir)?;

    let project_root = current_project.root_path().to_owned();

    let usages = exp_do_remove(current_project, publisher, name)?;
    print_removed(&usages);

    let usage_identifier = Identifier::from_pub_name(publisher, name);
    if !no_lock {
        lock_sync(
            ctx,
            config,
            no_sync,
            no_prune,
            resolution_opts,
            client,
            runtime,
            auth_policy,
            project_root,
            usage_identifier.as_str(),
        )?;
    }

    // Don't remove the project from config, as we don't properly support aliases
    // for PURL projects

    Ok(())
}

fn lock_sync<Policy: HTTPAuthentication>(
    ctx: ProjectContext,
    config: Config,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    project_root: Utf8PathBuf,
    iri: &str,
) -> Result<(), anyhow::Error> {
    let provided_iris = if resolution_opts.include_std {
        HashMap::default()
    } else {
        crate::known_std_libs()
    };
    let current_project = ctx.current_project.as_ref().unwrap();

    let alias_iris = if let Some(w) = &ctx.current_workspace {
        w.projects()
            .iter()
            .find(|p| Utf8Path::new(&p.path) == current_project.root_path())
            .map(|p| p.iris.clone())
    } else {
        None
    };

    let lockfile = project_root.join(DEFAULT_LOCKFILE_NAME);
    match fs::read_to_string(&lockfile) {
        Ok(l) => match Lock::from_str(&l) {
            Ok(mut lock) => {
                let info = current_project
                    .get_info()?
                    .ok_or(CliError::MissingProjectCurrentDir)?;
                match lock.remove_usage(info.publisher.as_deref(), &info.name, iri) {
                    Some(removed) => {
                        match removed.as_slice() {
                            [] => log::debug!(
                                "nothing removed from lockfile; dependency used by other project(s)"
                            ),
                            projects => {
                                log::debug!("projects removed from lockfile:");
                                for p in projects {
                                    log::debug!(
                                        "  publisher: {:?}, name: {}, first identifier: {:?}",
                                        p.publisher,
                                        p.name,
                                        p.identifiers.first()
                                    );
                                }
                                // Lock should not be canonicalized, as the goal is to make
                                // minimal necessary modifications to give a smaller diff
                                wrapfs::write(lockfile, lock.to_string())?;
                            }
                        }
                    }
                    None => {
                        log::warn!(
                            "lockfile was not modified, as it does not contain the current project;\n\
                            it is likely corrupt and should be regenerated by removing `sysand-lock.toml`\n\
                            and recreating it with `sysand lock`"
                        )
                    }
                }
                if !no_sync {
                    let mut env = crate::get_or_create_env(
                        ctx.env,
                        ctx.current_workspace.as_ref(),
                        ctx.current_project.as_ref(),
                        ctx.current_directory,
                    )?;
                    command_sync(
                        &lock,
                        project_root,
                        &mut env,
                        client,
                        &provided_iris,
                        runtime,
                        auth_policy,
                        ctx.current_workspace.as_ref(),
                        no_prune,
                    )?;
                }
            }
            // Include file path in errors
            Err(e) => bail!("invalid lockfile `{lockfile}`:\n{}", format_err(e)),
        },
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                resolve_deps(
                    no_sync,
                    no_prune,
                    resolution_opts,
                    &config,
                    client,
                    runtime,
                    auth_policy,
                    project_root,
                    alias_iris,
                    provided_iris,
                    ctx,
                )?;
            } else {
                bail!("failed to read lockfile `{lockfile}`: {}", format_err(e))
            }
        }
    }
    Ok(())
}

fn print_removed(usages: &[InterchangeProjectUsageRaw]) {
    let removed = "Removed";
    let header = sysand_core::style::get_style_config().header;
    for usage in usages {
        match usage {
            InterchangeProjectUsageRaw::Resource {
                resource,
                version_constraint,
            } => match version_constraint {
                Some(vc) => {
                    log::info!(
                        "{header}{removed:>12}{header:#} `{resource}` with version constraints `{vc}`"
                    );
                }
                None => {
                    log::info!("{header}{removed:>12}{header:#} `{resource}`");
                }
            },
            InterchangeProjectUsageRaw::Directory {
                dir: path,
                publisher,
                name,
            }
            | InterchangeProjectUsageRaw::KparPath {
                kpar_path: path,
                publisher,
                name,
            } => {
                log::info!("{header}{removed:>12}{header:#} `{publisher}/{name}` (path `{path}`)");
            }
        }
    }
}
