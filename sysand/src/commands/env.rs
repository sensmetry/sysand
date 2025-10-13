// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Result, anyhow, bail};

use sysand_core::{
    commands::{env::do_env_local_dir, lock::LockOutcome},
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    model::InterchangeProjectUsage,
    project::{
        ProjectRead, editable::EditableProject, local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
    },
    resolve::{
        ResolutionOutcome, ResolveRead,
        file::FileResolverProject,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::standard_resolver,
    },
};

use crate::{
    CliError,
    cli::{DependencyOptions, InstallOptions},
    commands::sync::command_sync,
};

pub fn command_env<P: AsRef<Path>>(path: P) -> Result<LocalDirectoryEnvironment> {
    Ok(do_env_local_dir(path)?)
}

// TODO: Factor out provided_iris logic
#[allow(clippy::too_many_arguments)]
pub fn command_env_install<S: AsRef<str>>(
    iri: S,
    version: Option<String>,
    install_opts: InstallOptions,
    dependency_opts: DependencyOptions,
    project_root: Option<PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let project_root = project_root.unwrap_or(std::env::current_dir()?);
    let mut env = crate::get_or_create_env(project_root.as_path())?;
    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let DependencyOptions {
        use_index,
        no_index,
        include_std,
    } = dependency_opts;

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

    let index_base_url = if no_index {
        None
    } else {
        let use_index: Result<Vec<_>, _> = use_index
            .iter()
            .map(|u| url::Url::parse(u.as_str()))
            .collect();
        Some(use_index?)
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
            index_base_url,
            runtime.clone(),
        ),
    );

    if no_deps {
        let outcome = resolver.resolve_read(&fluent_uri::Iri::from_str(iri.as_ref())?)?;
        // let outcome = resolver.resolve_read(&iri)?;
        if let ResolutionOutcome::Resolved(alternatives) = outcome {
            let storage = alternatives
                .into_iter()
                .filter_map(Result::ok)
                .find(|store| {
                    version.as_ref().is_none_or(|ver| {
                        store
                            .get_project()
                            .ok()
                            .and_then(|(opt, _)| opt)
                            .is_some_and(|proj| proj.version == *ver)
                    })
                })
                .ok_or(anyhow!(CliError::MissingProject(iri.as_ref().to_string())))?;
            sysand_core::commands::env::do_env_install_project(
                &iri,
                &storage,
                &mut env,
                allow_overwrite,
                allow_multiple,
            )?;
        } else {
            bail!(CliError::MissingProject(iri.as_ref().to_string()))
        }
    } else {
        let usages = vec![InterchangeProjectUsage {
            resource: fluent_uri::Iri::from_str(iri.as_ref())?,
            version_constraint: version
                .map(|v| semver::VersionReq::from_str(format!("^{}", v).as_str()))
                .transpose()?,
        }];

        let LockOutcome {
            lock,
            dependencies: _dependencies,
            inputs: _inputs,
        } = sysand_core::commands::lock::do_lock_extend(Lock::default(), usages, resolver)?;
        // Find if we added any std lib dependencies. This relies on `Lock::default()`
        // and `do_lock_extend()` to not read the existing lockfile, i.e. `lock` contains
        // only `iri` and `iri`'s dependencies.
        if !provided_iris.is_empty()
            && lock
                .projects
                .iter()
                .any(|x| x.iris.iter().any(|y| provided_iris.contains_key(y)))
        {
            crate::logger::warn_std_deps();
        }
        command_sync(
            lock,
            project_root,
            &mut env,
            client,
            &provided_iris,
            runtime,
        )?;
    }

    Ok(())
}

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_env_install_path<S: AsRef<str>>(
    iri: S,
    version: Option<String>,
    path: String,
    install_opts: InstallOptions,
    dependency_opts: DependencyOptions,
    project_root: Option<PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let project_root = project_root.unwrap_or(std::env::current_dir()?);
    let mut env = crate::get_or_create_env(project_root.as_path())?;
    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let DependencyOptions {
        use_index,
        no_index,
        include_std,
    } = dependency_opts;

    let project_path = PathBuf::from(&path);
    let project = if project_path.is_dir() {
        FileResolverProject::LocalSrcProject(LocalSrcProject { project_path })
    } else if project_path.is_file() {
        FileResolverProject::LocalKParProject(LocalKParProject::new_guess_root(project_path)?)
    } else {
        bail!("{} does not exist", project_path.display())
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

    let index_base_url = if no_index {
        None
    } else {
        let use_index: Result<Vec<_>, _> = use_index
            .iter()
            .map(|u| url::Url::parse(u.as_str()))
            .collect();
        Some(use_index?)
    };

    if let Some(version) = version {
        if version
            != project
                .get_info()?
                .ok_or(anyhow!("Missing project info"))?
                .version
        {
            bail!("Given version does not match project version")
        }
    }

    if no_deps {
        sysand_core::commands::env::do_env_install_project(
            iri,
            &project,
            &mut env,
            allow_overwrite,
            allow_multiple,
        )?;
    } else {
        // TODO: Fix this hack. Currently installing manually then turning project into Editable to
        // avoid errors when syncing. Lockfile generation should be configurable.
        sysand_core::commands::env::do_env_install_project(
            iri,
            &project,
            &mut env,
            allow_overwrite,
            allow_multiple,
        )?;
        let project = EditableProject::new(&path, project);

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
                Some(PathBuf::from(path)),
                None,
                Some(client.clone()),
                index_base_url,
                runtime.clone(),
            ),
        );
        let LockOutcome {
            lock,
            dependencies: _dependencies,
            inputs: _inputs,
        } = sysand_core::commands::lock::do_lock_projects(vec![project], resolver)?;
        command_sync(
            lock,
            project_root,
            &mut env,
            client,
            &provided_iris,
            runtime,
        )?;
    }

    Ok(())
}

pub fn command_env_uninstall<S: AsRef<str>>(
    iri: S,
    version: Option<S>,
    env: LocalDirectoryEnvironment,
) -> Result<()> {
    sysand_core::commands::env::do_env_uninstall(iri, version, env)?;
    Ok(())
}

pub fn command_env_list(env: Option<LocalDirectoryEnvironment>) -> Result<()> {
    let Some(env) = env else {
        bail!("Unable to identify environment to list.");
    };

    for (uri, version) in sysand_core::commands::env::do_env_list(env)? {
        println!("{uri} {}", version.unwrap_or("".to_string()));
    }
    Ok(())
}
