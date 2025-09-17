// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use sysand_core::{
    commands::env::do_env_local_dir,
    env::local_directory::LocalDirectoryEnvironment,
    project::{ProjectRead, local_src::LocalSrcProject},
    resolve::{ResolutionOutcome, ResolveRead, env::EnvResolver},
};

use crate::CliError;

pub fn command_env<P: AsRef<Path>>(path: P) -> Result<LocalDirectoryEnvironment> {
    Ok(do_env_local_dir(path)?)
}

fn get_store_index<S: AsRef<str>>(
    iri: S,
    version: Option<String>,
    index_path: PathBuf,
) -> Result<LocalSrcProject> {
    let resolver = EnvResolver {
        env: LocalDirectoryEnvironment {
            environment_path: index_path,
        },
    };
    let outcome = resolver.resolve_read_raw(&iri)?;
    if let ResolutionOutcome::Resolved(resolved) = outcome {
        resolved
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
            .ok_or(anyhow!(CliError::MissingProject(iri.as_ref().to_string())))
    } else {
        bail!(CliError::MissingProject(iri.as_ref().to_string()))
    }
}

pub fn command_env_install<S: AsRef<str>>(
    iri: S,
    version: Option<String>,
    env: &mut LocalDirectoryEnvironment,
    location: Option<String>,
    index: Option<String>,
    allow_overwrite: bool,
    allow_multiple: bool,
) -> Result<()> {
    let storage = match (location, index) {
        (Some(loc), _) => LocalSrcProject {
            project_path: Path::new(&loc).to_path_buf(),
        },
        (None, Some(path)) => {
            let index_path = Path::new(&path).to_path_buf();
            get_store_index(&iri, version, index_path)?
        }
        (None, None) => bail!("Must have either location or index for now"),
    };

    sysand_core::commands::env::do_env_install_project(
        iri,
        storage,
        env,
        allow_overwrite,
        allow_multiple,
    )?;
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
