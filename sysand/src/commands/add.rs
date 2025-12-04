// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::Result;

use sysand_core::{
    add::do_add,
    config::Config,
    lock::Lock,
    project::{local_src::LocalSrcProject, utils::wrapfs},
};

use crate::{CliError, cli::DependencyOptions, command_sync};

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_add(
    iri: String,
    versions_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    dependency_opts: DependencyOptions,
    config: &Config,
    current_project: Option<LocalSrcProject>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    let provided_iris = if !dependency_opts.include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(&iri) {
            crate::logger::warn_std(&iri);
            return Ok(());
        }
        sysml_std
    } else {
        HashMap::default()
    };

    do_add(&mut current_project, iri, versions_constraint)?;

    if !no_lock {
        crate::commands::lock::command_lock(
            PathBuf::from("."),
            dependency_opts,
            config,
            client.clone(),
            runtime.clone(),
        )?;

        if !no_sync {
            let mut env = crate::get_or_create_env(project_root.as_path())?;
            let lock = Lock::from_str(&wrapfs::read_to_string(
                project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME),
            )?)?;
            command_sync(
                lock,
                project_root,
                &mut env,
                client,
                &provided_iris,
                runtime,
            )?;
        }
    }

    Ok(())
}
