// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::{Result, anyhow, bail};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use sysand_core::{
    auth::HTTPAuthentication,
    commands::{env::do_env_local_dir, lock::LockOutcome},
    config::Config,
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    model::InterchangeProjectUsage,
    project::{
        ProjectRead, editable::EditableProject, local_kpar::LocalKParProject,
        local_src::LocalSrcProject, utils::wrapfs,
    },
    resolve::{
        file::FileResolverProject,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::standard_resolver,
    },
};

use crate::{
    DEFAULT_INDEX_URL,
    cli::{InstallOptions, ResolutionOptions},
    commands::sync::command_sync,
};

pub fn command_env<P: AsRef<Utf8Path>>(path: P) -> Result<LocalDirectoryEnvironment> {
    Ok(do_env_local_dir(path)?)
}

// TODO: Factor out provided_iris logic
#[allow(clippy::too_many_arguments)]
pub fn command_env_install<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    version: Option<String>,
    install_opts: InstallOptions,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: Option<Utf8PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let project_root = project_root.unwrap_or(wrapfs::current_dir()?);
    let mut env = crate::get_or_create_env(project_root.as_path())?;
    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std,
    } = resolution_opts;

    // TODO: should probably first check that current project exists
    let provided_iris = if !include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri.as_ref()) {
            crate::logger::warn_std(iri.as_ref());
            return Ok(());
        }
        sysml_std
    } else {
        HashMap::default()
    };

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    let mut memory_projects = HashMap::default();
    for (k, v) in &provided_iris {
        memory_projects.insert(fluent_uri::Iri::parse(k.clone()).unwrap(), v.to_vec());
    }

    // TODO: Move out the runtime
    let resolver = PriorityResolver::new(
        MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: memory_projects,
        },
        standard_resolver(
            None,
            None,
            Some(client.clone()),
            index_urls,
            runtime.clone(),
            auth_policy.clone(),
        ),
    );

    // TODO: don't use different root project resolution
    //       mechanisms depending on no_deps
    if no_deps {
        let (_version, storage) =
            crate::commands::clone::get_project_version(&iri, version, &resolver)?;
        sysand_core::commands::env::do_env_install_project(
            &iri,
            &storage,
            &mut env,
            allow_overwrite,
            allow_multiple,
        )?;
    } else {
        let usages = vec![InterchangeProjectUsage {
            resource: fluent_uri::Iri::from_str(iri.as_ref())?,
            version_constraint: version.map(|v| semver::VersionReq::parse(&v)).transpose()?,
        }];

        let LockOutcome {
            lock,
            dependencies: _dependencies,
        } = sysand_core::commands::lock::do_lock_extend(Lock::default(), usages, resolver)?;
        // Find if we added any std lib dependencies. This relies on `Lock::default()`
        // and `do_lock_extend()` to not read the existing lockfile, i.e. `lock` contains
        // only `iri` and `iri`'s dependencies.
        if !provided_iris.is_empty()
            && lock
                .projects
                .iter()
                .any(|x| x.identifiers.iter().any(|y| provided_iris.contains_key(y)))
        {
            crate::logger::warn_std_deps();
        }
        command_sync(
            &lock,
            project_root,
            false,
            &mut env,
            client,
            &provided_iris,
            runtime,
            auth_policy,
        )?;
    }

    Ok(())
}

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_env_install_path<S: AsRef<str>, Policy: HTTPAuthentication>(
    iri: S,
    version: Option<String>,
    path: Utf8PathBuf,
    install_opts: InstallOptions,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: Option<Utf8PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let project_root = project_root.unwrap_or(wrapfs::current_dir()?);
    let mut env = crate::get_or_create_env(project_root.as_path())?;
    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std,
    } = resolution_opts;

    let m = wrapfs::metadata(&path)?;
    let project = if m.is_dir() {
        FileResolverProject::LocalSrcProject(LocalSrcProject {
            project_path: path.as_str().into(),
        })
    } else if m.is_file() {
        FileResolverProject::LocalKParProject(LocalKParProject::new_guess_root(&path)?)
    } else {
        bail!("path `{path}` is neither a directory nor a file");
    };

    let provided_iris = if !include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri.as_ref()) {
            crate::logger::warn_std(iri.as_ref());
            return Ok(());
        }
        sysml_std
    } else {
        HashMap::default()
    };

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    if let Some(version) = version {
        let project_version = project
            .get_info()?
            .ok_or_else(|| anyhow!("missing project info"))?
            .version;
        if version != project_version {
            bail!("given version {version} does not match project version {project_version}")
        }
    }

    // TODO: Fix this hack. Currently installing manually then turning project into Editable to
    // avoid errors when syncing. Lockfile generation should be configurable.
    sysand_core::commands::env::do_env_install_project(
        iri,
        &project,
        &mut env,
        allow_overwrite,
        allow_multiple,
    )?;
    if !no_deps {
        let project = EditableProject::new(Utf8PathBuf::new(), project);

        let mut memory_projects = HashMap::default();
        for (k, v) in provided_iris.iter() {
            memory_projects.insert(fluent_uri::Iri::parse(k.clone()).unwrap(), v.to_vec());
        }

        // TODO: Move out the runtime
        let resolver = PriorityResolver::new(
            MemoryResolver {
                iri_predicate: AcceptAll {},
                projects: memory_projects,
            },
            standard_resolver(
                Some(path),
                None,
                Some(client.clone()),
                index_urls,
                runtime.clone(),
                auth_policy.clone(),
            ),
        );
        let LockOutcome {
            lock,
            dependencies: _dependencies,
        } = sysand_core::commands::lock::do_lock_projects([&project], resolver)?;
        command_sync(
            &lock,
            project_root,
            false,
            &mut env,
            client,
            &provided_iris,
            runtime,
            auth_policy,
        )?;
    }

    Ok(())
}

pub fn command_env_uninstall<S: AsRef<str>, Q: AsRef<str>>(
    iri: S,
    version: Option<Q>,
    env: LocalDirectoryEnvironment,
) -> Result<()> {
    sysand_core::commands::env::do_env_uninstall(iri, version, env)?;
    Ok(())
}

pub fn command_env_list(env: Option<LocalDirectoryEnvironment>) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify environment to list");
    };

    for (uri, version) in sysand_core::commands::env::do_env_list(env)? {
        println!("`{uri}` {}", version.unwrap_or("".to_string()));
    }
    Ok(())
}
