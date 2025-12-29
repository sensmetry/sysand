// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;

use sysand_core::{
    add::do_add,
    commands::lock::{LockOutcome, do_lock_extend},
    config::Config,
    lock::Lock,
    project::{local_src::LocalSrcProject, utils::wrapfs},
};

use crate::{
    CliError, cli::ResolutionOptions, command_sync, commands::lock::create_resolver, read_lockfile,
};

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_add<S: AsRef<str>>(
    iri: S,
    version_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    current_project: Option<LocalSrcProject>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    let provided_iris = if !resolution_opts.include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri.as_ref()) {
            crate::logger::warn_std(iri);
            return Ok(());
        }
        sysml_std
    } else {
        HashMap::default()
    };

    let usage_raw = sysand_core::model::InterchangeProjectUsageRaw {
        // TODO: fix the function to take String
        resource: iri.as_ref().to_owned(),
        version_constraint,
    };

    if !no_lock {
        let lockfile_path = project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);

        let resolver = create_resolver(
            &project_root,
            resolution_opts,
            config,
            provided_iris.clone(),
            client.clone(),
            runtime.clone(),
        )?;
        let current_lock = if !lockfile_path.is_file() {
            Lock::default()
        } else {
            read_lockfile(&lockfile_path)?
        };

        // Lock before adding to check if resource (and its required version)
        // can be found
        let usage = usage_raw.validate()?;
        let LockOutcome { lock, .. } = do_lock_extend(current_lock, [usage], resolver)?;

        do_add(&mut current_project, usage_raw)?;

        let lock = lock.canonicalize();
        wrapfs::write(lockfile_path, lock.to_string())?;

        if !no_sync {
            let mut env = crate::get_or_create_env(&project_root)?;
            command_sync(
                &lock,
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
