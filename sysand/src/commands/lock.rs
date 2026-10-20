// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::{
        EditableLocalSrcProject, LockOutcome, LockProjectError, do_lock_local_editable,
        do_lock_workspace,
    },
    config::Config,
    context::ProjectContext,
    lock::Lockfile,
    project::{any::AnyProject, memory::InMemoryProject, reference::ProjectReference},
    resolve::{
        ResolveRead,
        memory::{AcceptAll, AcceptResource, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    stdlib::known_std_libs,
    utils::ProvidedProjects,
    workspace::model_toml::{MemberChecks, Workspace},
};
use typed_path::Utf8UnixPath;

use crate::{cli::ResolutionOptions, default_index_location, get_overrides};

/// The resolver [`create_resolver`] builds: overrides, then provided
/// projects, then the standard file/env/index stack. Named so that callers
/// (the language bindings) can match the errors it produces.
pub type CliResolver<Policy> = PriorityResolver<
    PriorityResolver<
        MemoryResolver<AcceptResource, ProjectReference<AnyProject<Policy>>>,
        MemoryResolver<AcceptAll, InMemoryProject>,
    >,
    StandardResolver<Policy>,
>;

/// The error `resolve_lock`/`command_lock` fail with when the lock itself
/// fails (as opposed to configuration or context errors); reachable from
/// the returned `anyhow::Error` through `downcast_ref`.
pub type CliLockError<Policy> = LockProjectError<
    EditableLocalSrcProject,
    <CliResolver<Policy> as ResolveRead>::ProjectStorage,
    CliResolver<Policy>,
>;

/// Solve for a lockfile without writing it.
///
/// `extra_provided` are projects the caller itself provides (satisfied
/// without installing), merged with the standard libraries unless
/// `include_std` is set. The returned lock is canonical.
pub fn resolve_lock<P: AsRef<Utf8UnixPath>, Policy: HTTPAuthentication, R: AsRef<Utf8Path>>(
    path: P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: R,
    extra_provided: ProvidedProjects,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<sysand_core::lock::Lock> {
    let mut provided_iris = if resolution_opts.include_std {
        HashMap::default()
    } else {
        known_std_libs()
    };
    provided_iris.extend(extra_provided);
    let wrapped_resolver = create_resolver(
        resolution_opts,
        config,
        ctx,
        // TODO: avoid expensive clone here
        provided_iris.clone(),
        client,
        runtime,
        auth_policy,
    )?;

    let LockOutcome {
        lock,
        dependencies: _dependencies,
    } = do_lock_local_editable(
        &path,
        &project_root,
        // No alias IRIs: `model.toml` has no such key. A member is
        // identified by its `[project]` table, and `do_lock_workspace`
        // records `identity().lock_identifier()`.
        None,
        &provided_iris,
        wrapped_resolver,
        ctx,
    )?;

    Ok(lock.canonicalize())
}

/// Lock a whole `model.toml` workspace, and write the one lockfile at
/// its root.
///
/// Not `resolve_lock`/`command_lock`: the first is hardwired to
/// `do_lock_local_editable`, and the second writes to its `path`
/// argument, which is the *nominal editable path* of the project being
/// locked -- so passing the workspace root there would also change what
/// the lockfile records as editable.
pub fn command_lock_workspace<Policy: HTTPAuthentication>(
    root: &Utf8Path,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<sysand_core::lock::Lock> {
    // `SkipStrayState`: a member may hold an `output/` or a lockfile of
    // its own from an earlier run, and is still a member.
    // `do_lock_workspace`'s own contract asks for it.
    let workspace = Workspace::load_with(root, MemberChecks::SkipStrayState)?;

    let provided_iris = if resolution_opts.include_std {
        HashMap::default()
    } else {
        known_std_libs()
    };
    let wrapped_resolver = create_resolver(
        resolution_opts,
        config,
        ctx,
        provided_iris.clone(),
        client,
        runtime,
        auth_policy,
    )?;

    let LockOutcome {
        lock,
        dependencies: _,
    } = do_lock_workspace(&workspace, wrapped_resolver, &provided_iris, ctx)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let lockfile = Lockfile::new(root, lock.canonicalize());
    lockfile.write()?;
    Ok(lockfile.into_lock())
}

/// Generate a lockfile for `current_project` and write it.
pub fn command_lock<P: AsRef<Utf8UnixPath>, Policy: HTTPAuthentication, R: AsRef<Utf8Path>>(
    path: P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: R,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<sysand_core::lock::Lock> {
    let canonical = resolve_lock(
        &path,
        resolution_opts,
        config,
        project_root,
        ProvidedProjects::default(),
        client,
        runtime,
        auth_policy,
        ctx,
    )?;
    let lockfile = Lockfile::new(Utf8Path::new(path.as_ref().as_str()), canonical);
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
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std: _,
    } = resolution_opts;

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![default_index_location()], default_index))
    };

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
        )?,
    );

    Ok(wrapped_resolver)
}
