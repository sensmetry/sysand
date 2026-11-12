// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::{cli::ResolutionOptions, context::NotInProject, get_overrides};
use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::{
        EditableLocalSrcProject, LockProjectError, do_lock_local_editable,
        do_lock_local_editable_respelling, do_lock_workspace, do_lock_workspace_respelling,
    },
    config::Config,
    context::ProjectContext,
    lock::{Lock, Lockfile},
    project::{
        any::AnyProject, memory::InMemoryProject, reference::ProjectReference, utils::Identifier,
    },
    resolve::{
        ResolveRead,
        memory::{AcceptAll, AcceptResource, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    utils::ProvidedProjects,
};

/// The resolver [`create_resolver`] builds: overrides, then provided
/// projects, then the standard file/env/index stack. Named so the language
/// bindings can match its errors.
pub type CliResolver<Policy> = PriorityResolver<
    PriorityResolver<
        MemoryResolver<AcceptResource, ProjectReference<AnyProject<Policy>>>,
        MemoryResolver<AcceptAll, InMemoryProject>,
    >,
    StandardResolver<Policy>,
>;

/// The error [`lock_scope`] and the functions built on it fail with when
/// the lock itself fails (as opposed to configuration or context errors);
/// reachable from the returned `anyhow::Error` through `downcast_ref`.
pub type CliLockError<Policy> = LockProjectError<
    EditableLocalSrcProject,
    <CliResolver<Policy> as ResolveRead>::ProjectStorage,
    CliResolver<Policy>,
>;

/// Solve for the lockfile of `ctx`'s scope with `resolver`, without
/// writing it: the whole workspace in one solve when `ctx` is in one, and
/// the current project otherwise. The returned lock is canonical.
///
/// The spelling of the typed usage of `respelled` is not checked: it is
/// taken from the project the usage resolved to.
///
/// Fails with [`NotInProject`] outside both a workspace and a project.
pub fn lock_scope<Policy: HTTPAuthentication>(
    resolver: CliResolver<Policy>,
    provided_iris: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock> {
    // A single-member solve written over a workspace's shared lockfile
    // would drop every other member.
    if let Some(workspace) = ctx.workspace() {
        let lock = match respelled {
            None => do_lock_workspace(workspace, resolver, provided_iris, ctx),
            Some(respelled) => {
                do_lock_workspace_respelling(workspace, resolver, provided_iris, ctx, respelled)
            }
        }?;
        return Ok(lock.canonicalize());
    }
    let Some(project_root) = ctx.project_root() else {
        bail!(NotInProject {
            dir: ctx.current_directory.clone()
        });
    };
    // No alias IRIs: `model.toml` has no such key.
    let lock = match respelled {
        None => do_lock_local_editable(".", &project_root, None, provided_iris, resolver, ctx)?,
        Some(respelled) => do_lock_local_editable_respelling(
            ".",
            &project_root,
            None,
            provided_iris,
            resolver,
            ctx,
            respelled,
        )?,
    };
    Ok(lock.canonicalize())
}

/// [`lock_scope`] with the resolver [`create_resolver`] builds.
///
/// `extra_provided` are projects the caller itself provides (satisfied
/// without installing), added to [`ResolutionOptions::provided_iris`].
pub fn resolve_lock<Policy: HTTPAuthentication>(
    resolution_opts: ResolutionOptions,
    config: &Config,
    extra_provided: ProvidedProjects,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<Lock> {
    let provided_iris = resolution_opts.provided_iris(extra_provided);
    let resolver = create_resolver(
        resolution_opts,
        config,
        ctx,
        // TODO: avoid expensive clone here
        provided_iris.clone(),
        client,
        runtime,
        auth_policy,
    )?;
    lock_scope(resolver, &provided_iris, ctx, None)
}

/// [`resolve_lock`], written to the lockfile at `ctx`'s state root.
pub fn command_lock<Policy: HTTPAuthentication>(
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<Lock> {
    let lock = resolve_lock(
        resolution_opts,
        config,
        ProvidedProjects::default(),
        client,
        runtime,
        auth_policy,
        ctx,
    )?;
    let lockfile = Lockfile::new(ctx.state_root(), lock);
    lockfile.write()?;
    Ok(lockfile.into_lock())
}

/// A relative override path resolves against `ctx.config_root`.
pub fn create_resolver<Policy: HTTPAuthentication>(
    resolution_opts: ResolutionOptions,
    config: &Config,
    ctx: &ProjectContext,
    provided_usages: ProvidedProjects,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<
    PriorityResolver<
        PriorityResolver<
            MemoryResolver<
                AcceptResource,
                sysand_core::project::reference::ProjectReference<
                    sysand_core::project::any::AnyProject<Policy>,
                >,
            >,
            MemoryResolver<AcceptAll, InMemoryProject>,
        >,
        StandardResolver<Policy>,
    >,
    anyhow::Error,
> {
    let index_urls = resolution_opts.index_urls(config);

    let overrides = get_overrides(
        config,
        &ctx.config_root,
        &client,
        runtime.clone(),
        auth_policy.clone(),
    )?;

    let override_resolver = PriorityResolver::new(
        MemoryResolver::resources_only(overrides),
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: provided_usages,
        },
    );
    let wrapped_resolver = PriorityResolver::new(
        override_resolver,
        standard_resolver(
            // TODO: borrow?
            ctx.env.clone(),
            Some(client),
            index_urls,
            runtime,
            auth_policy,
            ctx.cancellation.clone(),
        )?,
    );

    Ok(wrapped_resolver)
}
