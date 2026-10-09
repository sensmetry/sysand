// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use reqwest_middleware::ClientWithMiddleware;
use sysand_core::{
    auth::HTTPAuthentication,
    commands::sync::SyncOutcome,
    config::{
        Config,
        local_fs::{CONFIG_FILE, remove_project_source_from_config},
    },
    context::ProjectContext,
    lock::{Lockfile, RemoveUsageOutcome},
    model::{InterchangeProjectUsageRaw, UsageRef},
    project::{ProjectRead as _, utils::Identifier},
    remove::do_remove,
};

use crate::{
    CliError,
    cli::{RemoveProjectLocatorArgs, ResolutionOptions},
    commands::{add::resolve_deps, lock::create_resolver, sync::command_sync},
    iri_or_path_to_iri,
};

#[expect(clippy::fn_params_excessive_bools)]
pub fn command_remove<Policy: HTTPAuthentication>(
    locator: RemoveProjectLocatorArgs,
    mut ctx: ProjectContext,
    config: Config,
    config_file: Option<Utf8PathBuf>,
    no_config: bool,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    client: ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let RemoveProjectLocatorArgs {
        identifier,
        iri,
        iri_path,
    } = locator;
    let resolved_iri;
    let usage = if let Some((publisher, name)) = &identifier {
        UsageRef::Typed(publisher.as_str(), name.as_str())
    } else {
        resolved_iri = iri_or_path_to_iri(iri, iri_path)?;
        UsageRef::Resource(resolved_iri.borrow())
    };

    let current_project = ctx
        .current_project
        .as_mut()
        .ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path().to_owned();
    let config_path =
        config_file.or_else(|| (!no_config).then(|| current_project.root_path().join(CONFIG_FILE)));

    // `.project.json` is not backed up here, since the failure to lock
    // or sync cannot logically be caused by the remove command itself
    // (unlike `add`), the failure must be pre-existing or transient
    // (i.e. lock/sync would have also failed even without the `remove`).
    // Therefore lock/sync failures should not revert the removal
    let removed = do_remove(current_project, usage)?;
    print_removed(&removed);

    // Identifiers must be derived from the removed usages, not from `usage`,
    // since a typed usage can be matched by a spelling that yields a
    // different identifier than the declared one
    let removed_identifiers: Vec<_> = removed
        .iter()
        .filter_map(Identifier::from_unvalidated_usage)
        .collect();
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
            &removed_identifiers,
        )?;
    }

    // This has to be done after the resolution
    // TODO: this is not always correct, as config file overrides also
    // affect transitive dependencies
    // Typed usages are not removed from config, as we don't properly support
    // aliases for PURL projects
    if let Some(path) = config_path {
        for usage in &removed {
            if let InterchangeProjectUsageRaw::Resource { resource, .. } = usage
                && let Ok(iri) = Iri::parse(resource.as_str())
            {
                remove_project_source_from_config(&path, iri)?;
            }
        }
    }

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
    removed_usages: &[Identifier],
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

    if let Some(mut lockfile) = Lockfile::try_read(&project_root)? {
        let info = current_project
            .get_info()?
            .ok_or(CliError::MissingProjectCurrentDir)?;
        let mut modified = false;
        let mut root_found = true;
        for id in removed_usages {
            match lockfile
                .lock_mut()
                .remove_usage(info.publisher.as_deref(), &info.name, id)
            {
                RemoveUsageOutcome::RootNotFound => {
                    root_found = false;
                    break;
                }
                RemoveUsageOutcome::UsageNotFound => {
                    log::debug!("usage `{id}` not present in lockfile");
                }
                RemoveUsageOutcome::Removed { pruned } => {
                    modified = true;
                    if pruned.is_empty() {
                        log::debug!(
                            "no projects pruned from lockfile for `{id}`; \
                            dependency used by other project(s)"
                        );
                    } else {
                        log::debug!("projects pruned from lockfile for `{id}`:");
                        for p in &pruned {
                            log::debug!(
                                "  publisher: {:?}, name: {}, first identifier: {:?}",
                                p.publisher,
                                p.name,
                                p.identifiers.first()
                            );
                        }
                    }
                }
            }
        }
        if !root_found {
            log::warn!(
                "lockfile was not modified, as it does not contain the current project;\n\
                it is likely corrupt and should be regenerated by removing `sysand-lock.toml`\n\
                and recreating it with `sysand lock`"
            );
        } else if modified {
            // Lock should not be canonicalized, as the goal is to make
            // minimal necessary modifications to give a smaller diff
            lockfile.write()?;
        }
        if !no_sync {
            let mut env = crate::get_or_create_env(
                ctx.env,
                ctx.current_workspace.as_ref(),
                ctx.current_project.as_ref(),
                ctx.current_directory,
            )?;
            command_sync(
                lockfile.lock(),
                project_root,
                &mut env,
                client,
                &provided_iris,
                runtime,
                auth_policy,
                ctx.current_workspace.as_ref(),
                no_prune,
                &mut SyncOutcome::default(),
            )?;
        }
    } else {
        let resolver = create_resolver(
            resolution_opts,
            &config,
            &project_root,
            &ctx,
            provided_iris.clone(),
            client.clone(),
            runtime.clone(),
            auth_policy.clone(),
        )?;
        resolve_deps(
            no_sync,
            no_prune,
            resolver,
            client,
            runtime,
            auth_policy,
            project_root,
            alias_iris,
            provided_iris,
            ctx,
        )?;
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
            InterchangeProjectUsageRaw::Index {
                publisher,
                name,
                version_constraint,
            } => {
                log::info!(
                    "{header}{removed:>12}{header:#} `{publisher}/{name}` with version constraints `{version_constraint}`"
                );
            }
        }
    }
}
