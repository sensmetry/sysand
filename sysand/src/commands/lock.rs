// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::{DEFAULT_LOCKFILE_NAME, LockOutcome, do_lock_local_editable},
    config::Config,
    env::local_directory::DEFAULT_ENV_NAME,
    project::{memory::InMemoryProject, utils::wrapfs},
    resolve::{
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    stdlib::known_std_libs,
    workspace::Workspace,
};

use crate::{DEFAULT_INDEX_URL, cli::ResolutionOptions};

/// Generate a lockfile for `current_project`.
/// `path` must be relative to workspace root.
// TODO: this will not work properly if run in subdir of workspace,
// as `path` will then refer to a deeper subdir
pub fn command_lock<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    path: P,
    current_workspace: Option<Workspace>,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<sysand_core::lock::Lock> {
    assert!(path.as_ref().is_relative(), "{}", path.as_ref());

    let provided_iris = if !resolution_opts.include_std {
        known_std_libs()
    } else {
        HashMap::default()
    };
    let wrapped_resolver = create_resolver(
        &path,
        resolution_opts,
        config,
        provided_iris,
        client,
        runtime,
        auth_policy,
    )?;

    let alias_iris = if let Some(w) = current_workspace {
        w.projects()
            .iter()
            .find(|p| Utf8Path::new(&p.path) == path.as_ref())
            .map(|p| p.iris.clone())
    } else {
        None
    };
    let LockOutcome {
        lock,
        dependencies: _dependencies,
    } = do_lock_local_editable(&path, alias_iris, wrapped_resolver)?;

    let canonical = lock.canonicalize();
    wrapfs::write(
        path.as_ref().join(DEFAULT_LOCKFILE_NAME),
        canonical.to_string(),
    )?;

    Ok(canonical)
}

pub fn create_resolver<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    path: &P,
    resolution_opts: ResolutionOptions,
    config: &Config,
    provided_iris: HashMap<String, Vec<InMemoryProject>>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<
    PriorityResolver<MemoryResolver<AcceptAll, InMemoryProject>, StandardResolver<Policy>>,
    anyhow::Error,
> {
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std: _,
    } = resolution_opts;

    let cwd = wrapfs::current_dir()?;

    let local_env_path = path.as_ref().join(DEFAULT_ENV_NAME);

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    // TODO: add fn next to known_std_libs() to get this structure directly
    // it is created in most? all? places where `known_std_libs()` is used
    let mut memory_projects = HashMap::default();
    for (k, v) in provided_iris {
        memory_projects.insert(fluent_uri::Iri::parse(k).unwrap(), v);
    }

    let wrapped_resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: memory_projects,
        },
        standard_resolver(
            Some(cwd),
            if local_env_path.is_dir() {
                Some(local_env_path)
            } else {
                None
            },
            Some(client),
            index_urls,
            runtime,
            auth_policy,
        ),
    );
    Ok(wrapped_resolver)
}
