// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::CliError;

use anstream::println;
use anyhow::{Result, bail};
use semver::VersionReq;
use sysand_core::{
    context::ProjectContext,
    env::{local_directory::LocalDirectoryEnvironment, null::NullEnvironment},
    project::ProjectRead,
    sources::{Dependencies, do_sources_local_src_project_no_deps, resolve_dependencies},
};

use sysand_core::env::ReadEnvironment;

pub fn command_sources_env<S: AsRef<str>>(
    iri: S,
    version: Option<VersionReq>,
    no_own: bool,
    dependencies: Dependencies,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let Some(env) = env else {
        bail!("unable to identify local environment");
    };

    let mut projects = env.candidate_projects(&iri)?.into_iter();

    let Some(project) = (match &version {
        // No version constraints, so choose the first candidate
        None => projects.next(),
        Some(vr) => loop {
            if let Some(candidate) = projects.next() {
                if let Some(v) = candidate
                    .version()?
                    .and_then(|x| semver::Version::parse(&x).ok())
                    && vr.matches(&v)
                {
                    break Some(candidate);
                }
            } else {
                break None;
            }
        },
    }) else {
        match version {
            Some(vr) => bail!(
                "unable to find project `{}` ({}) in local environment",
                iri.as_ref(),
                vr
            ),
            None => bail!(
                "unable to find project `{}` in local environment",
                iri.as_ref()
            ),
        }
    };

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&project, true)? {
            println!("{}", src_path);
        }
    }

    if dependencies != Dependencies::None {
        let Some(info) = project.get_info()? else {
            bail!("project is missing project information")
        };

        if dependencies == Dependencies::Deps {
            crate::logger::warn_std_deps();
        }
        for dep in resolve_dependencies(info.validate()?.usage, env, dependencies)? {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path);
            }
        }
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
    let info = info.validate()?;

    if !no_own {
        for src_path in do_sources_local_src_project_no_deps(&current_project, true)? {
            println!("{}", src_path);
        }
    }

    if dependencies != Dependencies::None {
        let deps = match ctx.env {
            Some(env) => resolve_dependencies(info.usage, env, dependencies)?,
            None => {
                let env = NullEnvironment::new();
                resolve_dependencies(info.usage, env, dependencies)?
            }
        };

        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path);
            }
        }
    }

    Ok(())
}
