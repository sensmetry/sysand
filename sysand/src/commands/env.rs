// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, anyhow, bail};

use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;

use semver::VersionReq;
use sysand_core::{
    auth::HTTPAuthentication,
    commands::{env::do_env_local_dir, lock::DeclaredBy, sync::SyncOutcome},
    config::Config,
    context::ProjectContext,
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    model::InterchangeProjectUsage,
    project::{
        ProjectDefinition, ProjectFormat, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        utils::wrapfs,
    },
    resolve::{
        ResolutionInfo,
        file::FileResolverProject,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::standard_resolver,
    },
    utils::SP,
};
use typed_path::Utf8UnixPathBuf;

use crate::{
    cli::{InstallOptions, ResolutionOptions},
    commands::sync::command_sync,
    get_overrides,
};

pub fn command_env<P: AsRef<Utf8Path>>(path: P) -> Result<LocalDirectoryEnvironment> {
    let env = do_env_local_dir(path)?;
    Ok(env)
}

// TODO: Factor out provided_iris logic
pub fn command_env_install<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    version_constraint: Option<VersionReq>,
    install_opts: InstallOptions,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: Option<Utf8PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    mut ctx: ProjectContext,
) -> Result<()> {
    let project_root = project_root.unwrap_or_else(|| ctx.current_directory.clone());
    let env = crate::get_or_create_env(&mut ctx)?;
    ctx.env = Some(env);

    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let include_std = resolution_opts.include_std;

    // TODO: should probably first check that current project exists
    let provided_usages = if include_std {
        HashMap::default()
    } else {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri.as_ref()) {
            warn_std_install(iri.as_ref());
            return Ok(());
        }
        sysml_std
    };

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
            projects: provided_usages.clone(),
        },
    );
    // TODO: Move out the runtime
    let resolver = PriorityResolver::new(
        override_resolver,
        standard_resolver(
            None,
            Some(client.clone()),
            index_urls,
            runtime.clone(),
            auth_policy.clone(),
            ctx.cancellation.clone(),
        )?,
    );

    // TODO: don't use different root project resolution
    //       mechanisms depending on no_deps
    if no_deps {
        let id = iri.to_string();
        let resolve = ResolutionInfo::iri(iri);
        let (version, storage) =
            crate::commands::clone::get_project_version(&resolve, version_constraint, &resolver)?;
        // TODO: install `model.toml` projects, rewriting path usages as a build does.
        if storage.format()? == ProjectFormat::Toml {
            bail!("`{resolve}` is a `model.toml` project, which cannot be copied yet");
        }
        sysand_core::commands::env::do_env_install_project(
            id,
            &version.to_string(),
            &storage,
            Some(storage.checksum_canonical_variant()?),
            // Initialized above
            &mut ctx.env.unwrap(),
            allow_overwrite,
            allow_multiple,
        )?;
    } else {
        let usages = [(
            InterchangeProjectUsage::Resource {
                resource: iri,
                version_constraint,
            },
            DeclaredBy::Input("the command line".to_owned()),
        )];

        let lock = sysand_core::commands::lock::do_lock_extend(
            Lock::default(),
            usages,
            resolver,
            &provided_usages,
            &ctx,
        )?;
        // Find if we added any std lib dependencies. This relies on `Lock::default()`
        // and `do_lock_extend()` to not read the existing lockfile, i.e. `lock` contains
        // only `iri` and `iri`'s dependencies.
        // This is unreachable if `iri` is an std lib, so this warning will not duplicate
        // the above one
        if !provided_usages.is_empty()
            && lock.projects.iter().any(|x| {
                x.identifiers
                    .iter()
                    .any(|y| provided_usages.contains_key(y.as_str()))
            })
        {
            // TODO: this could be more helpful, currently it suggests `--include-std`,
            // but that by itself will not work, since the project will be already installed,
            // so `sysand env install <original iri> --allow-overwrite --include-std` is needed
            crate::logger::note_std_deps_no_install();
        }
        command_sync(
            &lock,
            project_root,
            // The lock holds a resolved IRI and its dependencies, never a member.
            None,
            &mut ctx.env.unwrap(),
            client,
            &provided_usages,
            runtime,
            auth_policy,
            // Since this explicitly installs into env, it would not make sense to prune
            true,
            ctx.cancellation,
            &mut SyncOutcome::default(),
        )?;
    }

    Ok(())
}

// TODO: Collect common arguments
pub fn command_env_install_path<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    version: Option<String>,
    path: Utf8PathBuf,
    install_opts: InstallOptions,
    resolution_opts: ResolutionOptions,
    config: &Config,
    project_root: Option<Utf8PathBuf>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    mut ctx: ProjectContext,
) -> Result<()> {
    let project_root = project_root.unwrap_or_else(|| ctx.current_directory.clone());
    let env = crate::get_or_create_env(&mut ctx)?;
    ctx.env = Some(env);

    let InstallOptions {
        allow_overwrite,
        allow_multiple,
        no_deps,
    } = install_opts;
    let include_std = resolution_opts.include_std;

    let metadata = wrapfs::metadata(&path)?;
    let project = if metadata.is_dir() {
        FileResolverProject::LocalSrcProject(sysand_core::workspace::model_toml::project_at_path(
            path.as_str(),
            // Provide an empty nominal path to satisfy lock. It won't be used for actual
            // syncing, as the project is installed manually
            Some(Utf8UnixPathBuf::new()),
        )?)
    } else if metadata.is_file() {
        FileResolverProject::LocalKParProject(LocalKParProject::new_access(
            &path,
            KparInnerPath::Guess,
            Some(path.as_str().into()),
        ))
    } else {
        bail!("path `{path}` is neither a directory nor a file");
    };

    let provided_usages = if include_std {
        HashMap::default()
    } else {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri.as_ref()) {
            warn_std_install(&iri);
            return Ok(());
        }
        sysml_std
    };

    let index_urls = resolution_opts.index_urls(config);

    let definition = project
        .get_project()?
        .ok_or_else(|| anyhow!("missing project info"))?;
    // TODO: install `model.toml` projects, rewriting path usages as a build does.
    if let ProjectDefinition::Toml(manifest) = &definition {
        bail!(
            "`{}` is a `model.toml` project, which cannot be copied yet",
            manifest.name.as_str()
        );
    }
    let project_version = definition.version();
    if let Some(version) = version
        && version != project_version
    {
        bail!("given version {version} does not match project version {project_version}")
    }

    // TODO: Fix this hack. Manual installation needed to respect `allow_overwrite`/`allow_multiple`.
    // Lockfile generation should be configurable. How to handle allow_overwrite/allow_multiple
    // for dependencies? How about when syncing (e.g. after `add`)?
    sysand_core::commands::env::do_env_install_project(
        iri.as_str(),
        &project_version,
        &project,
        Some(project.checksum_canonical_variant()?),
        ctx.env.as_mut().unwrap(),
        allow_overwrite,
        allow_multiple,
    )?;
    if !no_deps {
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
                projects: provided_usages.clone(),
            },
        );
        // TODO: Move out the runtime
        let resolver = PriorityResolver::new(
            override_resolver,
            standard_resolver(
                None,
                Some(client.clone()),
                index_urls,
                runtime.clone(),
                auth_policy.clone(),
                ctx.cancellation.clone(),
            )?,
        );
        let mut lock = sysand_core::commands::lock::do_lock_projects(
            [(Some(vec![iri]), &project)],
            resolver,
            &provided_usages,
            &ctx,
        )?;
        // FIXME: part of hack above, the project is already installed
        // If it's not removed from lock here, sync will try to install it again,
        // and fail because of missing sources path
        lock.projects.swap_remove(0);
        command_sync(
            &lock,
            project_root,
            // The lock holds a resolved IRI and its dependencies, never a member.
            None,
            &mut ctx.env.unwrap(),
            client,
            &provided_usages,
            runtime,
            auth_policy,
            // Since this explicitly installs into env, it would not make sense to prune
            true,
            ctx.cancellation.clone(),
            &mut SyncOutcome::default(),
        )?;
    }

    Ok(())
}

pub fn command_env_uninstall<S: AsRef<str>, V: AsRef<str>>(
    iri: S,
    version: Option<V>,
    mut env: LocalDirectoryEnvironment,
) -> Result<()> {
    sysand_core::commands::env::do_env_uninstall(&iri, version.as_ref(), &mut env)?;
    env.write()?;

    Ok(())
}

pub fn command_env_list(env: Option<LocalDirectoryEnvironment>) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify environment to list");
    };

    let mut listed: Vec<(String, String)> = sysand_core::commands::env::do_env_list(&env)?
        .into_iter()
        .map(|(uri, version)| (uri, version.unwrap_or_default()))
        .collect();
    if crate::output::is_json() {
        // A defined order for the document; the text form keeps the
        // environment's.
        listed.sort();
        for (iri, version) in &listed {
            crate::output::push(
                "projects",
                serde_json::json!({ "iri": iri, "version": version }),
            );
        }
        return Ok(());
    }
    for (uri, version) in listed {
        println!("`{uri}` {version}");
    }
    Ok(())
}

fn warn_std_install(iri: impl AsRef<str>) {
    log::warn!(
        "it's not recommended to install SysML v2/KerML standard
        {SP:>8} library package `{}`,\n\
        {SP:>8} as installing it may break other tooling, since standard library\n\
        {SP:>8} is usually provided by default, and the tool will therefore see two\n\
        {SP:>8} conflicting copies of the same library;\n\
        {SP:>8} to proceed anyway, pass `--include-std`",
        iri.as_ref()
    );
}
