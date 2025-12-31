// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;

use sysand_core::{
    add::do_add,
    commands::lock::{DEFAULT_LOCKFILE_NAME, LockOutcome, do_lock_local_editable},
    config::Config,
    model::InterchangeProjectUsageRaw,
    project::{local_src::LocalSrcProject, utils::wrapfs},
};

use crate::{CliError, cli::ResolutionOptions, command_sync, commands::lock::create_resolver};

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_add(
    iri: fluent_uri::Iri<String>,
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

    let usage_raw = InterchangeProjectUsageRaw {
        resource: iri.into_string(),
        version_constraint,
    };

    if !no_lock {
        let info_path = current_project.project_path.join(".project.json");
        let info_backup = wrapfs::read_to_string(&info_path)?;
        do_add(&mut current_project, &usage_raw)?;

        match resolve_deps(
            no_sync,
            resolution_opts,
            config,
            client,
            runtime,
            &current_project.project_path,
            provided_iris,
        ) {
            Ok(_) => Ok(()),
            Err(e) => {
                // Restore old info
                wrapfs::write(&info_path, info_backup)?;
                Err(e)
            }
        }
    } else {
        do_add(&mut current_project, &usage_raw)?;
        Ok(())
    }
}

fn resolve_deps<P: AsRef<Path>>(
    no_sync: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    project_root: P,
    provided_iris: HashMap<String, Vec<sysand_core::project::memory::InMemoryProject>>,
) -> Result<(), anyhow::Error> {
    let resolver = create_resolver(
        &project_root,
        resolution_opts,
        config,
        provided_iris.clone(),
        client.clone(),
        runtime.clone(),
    )?;
    let LockOutcome { lock, .. } = do_lock_local_editable(&project_root, resolver)?;
    let lock = lock.canonicalize();
    wrapfs::write(
        project_root.as_ref().join(DEFAULT_LOCKFILE_NAME),
        lock.to_string(),
    )?;
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
    Ok(())
}
