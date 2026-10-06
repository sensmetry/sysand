// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    CliError,
    cli::{Dependencies, EnvProjectLocatorArgs},
};

use anstream::println;
use anyhow::{Context as _, Result, bail};
use semver::VersionReq;
use sysand_core::{
    context::ProjectContext,
    env::{local_directory::LocalDirectoryEnvironment, null::NullEnvironment},
    model::UsageRef,
    project::ProjectRead as _,
    sources::{do_sources_env, do_sources_local_src_project_no_deps, resolve_dependencies},
};

pub fn command_sources_env(
    locator: EnvProjectLocatorArgs,
    version: Option<VersionReq>,
    no_own: bool,
    dependencies: Dependencies,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify local environment");
    };

    let project = match &locator {
        EnvProjectLocatorArgs {
            identifier: Some((publisher, name)),
            iri: None,
        } => UsageRef::Typed(publisher.as_str(), name.as_str()),
        EnvProjectLocatorArgs {
            identifier: None,
            iri: Some(iri),
        } => UsageRef::Resource(iri.borrow()),
        _ => unreachable!(),
    };

    let sources = do_sources_env(env, project, version.as_ref(), no_own, dependencies.into())?;
    for src_path in sources {
        println!("{src_path}");
    }

    Ok(())
}

pub fn command_sources_project(
    no_own: bool,
    dependencies: Dependencies,
    ctx: ProjectContext,
) -> Result<()> {
    let current_project = ctx
        .current_project
        .ok_or(CliError::MissingProjectCurrentDir)?;
    // TODO: Better bail early?
    let Some(info) = current_project.get_info()? else {
        bail!("project is missing project information")
    };
    let info = info.validate().with_context(|| {
        format!(
            "project `{}` {} has invalid metadata",
            info.name, info.version
        )
    })?;

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&current_project, true)? {
            println!("{}", src_path);
        }
    }

    if dependencies != Dependencies::None {
        let dependencies = dependencies.into();
        let deps = if let Some(env) = ctx.env {
            resolve_dependencies(info.usage, env, dependencies)?
        } else {
            let env = NullEnvironment::new();
            resolve_dependencies(info.usage, env, dependencies)?
        };

        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path);
            }
        }
    }

    Ok(())
}
