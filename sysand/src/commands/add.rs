// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;

use sysand_core::{
    add::do_add,
    commands::lock::{LockOutcome, do_lock_extend},
    lock::Lock,
    project::{local_src::LocalSrcProject, utils::wrapfs},
};

use crate::{
    CliError,
    cli::DependencyOptions,
    command_sync,
    commands::lock::{create_resolver, handle_lock_error},
    read_lockfile,
};

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_add(
    iri: String,
    version_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    dependency_opts: DependencyOptions,
    current_project: Option<LocalSrcProject>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let DependencyOptions {
        use_index,
        no_index,
        include_std,
    } = dependency_opts;
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    let provided_iris = if !include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(&iri) {
            crate::logger::warn_std(&iri);
            return Ok(());
        }
        sysml_std
    } else {
        HashMap::default()
    };

    let usage_raw = sysand_core::model::InterchangeProjectUsageRaw {
        resource: iri,
        version_constraint,
    };
    if !no_lock {
        let lockfile_path = project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);
        let index_base_urls = if no_index { None } else { Some(use_index) };
        let resolver = create_resolver(
            &project_root,
            client.clone(),
            index_base_urls,
            runtime.clone(),
            &provided_iris,
        )?;
        let current_lock = if !lockfile_path.is_file() {
            Lock::default()
        } else {
            read_lockfile(&lockfile_path)?
        };

        // Lock before adding to check if resource (and its required version)
        // can be found
        let usage = usage_raw.validate()?;
        let LockOutcome { lock, .. } =
            do_lock_extend(current_lock, std::iter::once(usage), resolver)
                .map_err(handle_lock_error)?;
        let lock = lock.canonicalize();
        wrapfs::write(lockfile_path, lock.to_string())?;

        do_add(&mut current_project, usage_raw)?;

        if !no_sync {
            let mut env = crate::get_or_create_env(project_root.as_path())?;
            command_sync(
                lock,
                project_root,
                &mut env,
                client,
                &provided_iris,
                runtime,
            )?;
        }
    } else {
        do_add(&mut current_project, usage_raw)?;
    }

    Ok(())
}
