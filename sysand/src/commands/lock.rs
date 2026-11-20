// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use anyhow::Result;

use crate::{cli::ResolutionOptions, context::NotInProject, get_overrides};
use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::{
        EditableLocalSrcProject, LockProjectError, do_lock_local_editable, do_lock_workspace,
    },
    config::Config,
    context::ProjectContext,
    env::local_directory::LocalDirectoryEnvironment,
    lock::{Lock, Lockfile, LockfileError},
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
/// the lock itself fails; carried by [`LockFailure::Lock`].
pub type CliLockError<Policy> = LockProjectError<
    EditableLocalSrcProject,
    <CliResolver<Policy> as ResolveRead>::ProjectStorage,
    CliResolver<Policy>,
>;

/// Why locking failed, for a caller that tells the kinds apart.
///
/// Any other error converts into [`LockFailure::Other`]. Not convertible
/// into [`anyhow::Error`] by `From`, so that the conversion from it can be
/// that general: see [`LockFailure::into_error`].
#[derive(Debug)]
pub enum LockFailure<Policy: HTTPAuthentication> {
    /// The existing lockfile could not be read.
    Lockfile(LockfileError),
    /// The lock itself failed.
    Lock(Box<CliLockError<Policy>>),
    Other(anyhow::Error),
}

impl<Policy: HTTPAuthentication> LockFailure<Policy> {
    /// The error as a plain [`anyhow::Error`].
    pub fn into_error(self) -> anyhow::Error {
        match self {
            Self::Lockfile(error) => error.into(),
            Self::Lock(error) => error.into(),
            Self::Other(error) => error,
        }
    }
}

impl<Policy: HTTPAuthentication, E: Into<anyhow::Error>> From<E> for LockFailure<Policy> {
    fn from(error: E) -> Self {
        Self::Other(error.into())
    }
}

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
) -> Result<Lock, LockFailure<Policy>> {
    // A single-member solve written over a workspace's shared lockfile
    // would drop every other member.
    if let Some(workspace) = ctx.workspace() {
        let lock = do_lock_workspace(workspace, resolver, provided_iris, ctx, respelled)
            .map_err(|error| LockFailure::Lock(Box::new(error)))?;
        return Ok(lock.canonicalize());
    }
    let Some(project_root) = ctx.project_root() else {
        return Err(NotInProject {
            dir: ctx.current_directory.clone(),
        }
        .into());
    };
    // No alias IRIs: `model.toml` has no such key.
    let lock = do_lock_local_editable(&project_root, None, provided_iris, resolver, ctx, respelled)
        .map_err(|error| LockFailure::Lock(Box::new(error)))?;
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
) -> Result<Lock, LockFailure<Policy>> {
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
) -> Result<Lock, LockFailure<Policy>> {
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
) -> Result<CliResolver<Policy>> {
    // TODO: borrow?
    create_resolver_with_env(
        ctx.env.clone(),
        resolution_opts,
        config,
        ctx,
        provided_usages,
        client,
        runtime,
        auth_policy,
    )
}

/// [`create_resolver`], resolving installed projects from `env` in place
/// of `ctx`'s environment.
#[expect(
    clippy::too_many_arguments,
    reason = "takes each independent input as its own parameter"
)]
pub fn create_resolver_with_env<Policy: HTTPAuthentication>(
    env: Option<LocalDirectoryEnvironment>,
    resolution_opts: ResolutionOptions,
    config: &Config,
    ctx: &ProjectContext,
    provided_usages: ProvidedProjects,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<CliResolver<Policy>> {
    let index_urls = resolution_opts.index_urls(config);

    let overrides = get_overrides(config, ctx, &client, runtime.clone(), auth_policy.clone())?;

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
            env,
            Some(client),
            index_urls,
            runtime,
            auth_policy,
            ctx.cancellation.clone(),
        ),
    );

    Ok(wrapped_resolver)
}
