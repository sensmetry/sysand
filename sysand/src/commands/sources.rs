// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::CliError;

use anyhow::{Result, bail};
use sysand_core::{
    env::{local_directory::LocalDirectoryEnvironment, null::NullEnvironment},
    project::ProjectRead,
    sources::{do_sources_local_src_project_no_deps, find_project_dependencies},
};

use semver::VersionReq;
use sysand_core::env::ReadEnvironment;

pub fn command_sources_env(
    iri: String,
    version: Option<VersionReq>,
    include_deps: bool,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let Some(env) = env else {
        bail!("Unable to identify local environment");
    };

    let mut projects = env.candidate_projects(&iri)?.into_iter();

    let Some(project) = (match &version {
        None => projects.next(),
        Some(vr) => loop {
            if let Some(candidate) = projects.next() {
                if let Some(v) = candidate
                    .version()?
                    .and_then(|x| semver::Version::parse(&x).ok())
                {
                    if vr.matches(&v) {
                        break Some(candidate);
                    }
                }
            } else {
                break None;
            }
        },
    }) else {
        match version {
            Some(vr) => bail!(
                "unable to find project {} ({}) in local environment",
                iri,
                vr
            ),
            None => bail!("unable to find project {} in local environment", iri),
        }
    };

    for src_path in do_sources_local_src_project_no_deps(&project, true)? {
        println!("{}", src_path.display());
    }

    if include_deps {
        let Some(info) = project.get_info()? else {
            bail!("project is missing project information")
        };

        for dep in find_project_dependencies(info.validate()?.usage, env)? {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path.display());
            }
        }
    }

    Ok(())
}

pub fn command_sources_project(
    include_deps: bool,
    current_project: Option<sysand_core::project::local_src::LocalSrcProject>,
    env: Option<LocalDirectoryEnvironment>,
) -> Result<()> {
    let current_project =
        current_project.ok_or(CliError::MissingProject("in current directory".to_string()))?;

    for src_path in do_sources_local_src_project_no_deps(&current_project, true)? {
        println!("{}", src_path.display());
    }

    if include_deps {
        // TODO: Better bail early?
        let Some(info) = current_project.get_info()? else {
            bail!("project is missing project information")
        };

        let deps = match env {
            Some(env) => find_project_dependencies(info.validate()?.usage, env)?,
            None => {
                let env = NullEnvironment::new();
                find_project_dependencies(info.validate()?.usage, env)?
            }
        };

        for dep in deps {
            for src_path in do_sources_local_src_project_no_deps(&dep, true)? {
                println!("{}", src_path.display());
            }
        }
    }

    Ok(())
}
