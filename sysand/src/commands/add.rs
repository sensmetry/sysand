// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::Result;

use sysand_core::{
    add::do_add,
    auth::HTTPAuthentication,
    config::{Config, local_fs::add_project_source_to_config},
    lock::Lock,
    project::{local_src::LocalSrcProject, utils::wrapfs},
};

use crate::{
    CliError,
    cli::{ProjectSourceOptions, ResolutionOptions},
    command_sync,
};

// TODO: Collect common arguments
#[allow(clippy::too_many_arguments)]
pub fn command_add<S: AsRef<str>, Policy: HTTPAuthentication>(
    iri: S,
    versions_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    resolution_opts: ResolutionOptions,
    source_opts: ProjectSourceOptions,
    config: &Config,
    current_project: Option<LocalSrcProject>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    if let Some(src_path) = source_opts.local_src {
        add_project_source_to_config(
            &project_root,
            &iri,
            &sysand_core::lock::Source::LocalSrc { src_path: src_path.into() },
        )?;
    } else if let Some(kpar_path) = source_opts.local_kpar {
        add_project_source_to_config(
            &project_root,
            &iri,
            &sysand_core::lock::Source::LocalKpar { kpar_path: kpar_path.into() },
        )?;
    } else if let Some(remote_src) = source_opts.remote_src {
        add_project_source_to_config(
            &project_root,
            &iri,
            &sysand_core::lock::Source::RemoteSrc { remote_src },
        )?;
    } else if let Some(remote_kpar) = source_opts.remote_kpar {
        add_project_source_to_config(
            &project_root,
            &iri,
            &sysand_core::lock::Source::RemoteKpar {
                remote_kpar,
                remote_kpar_size: None,
            },
        )?;
    }

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

    do_add(&mut current_project, iri, versions_constraint)?;

    if !no_lock {
        crate::commands::lock::command_lock(
            ".",
            resolution_opts,
            config,
            &project_root,
            client.clone(),
            runtime.clone(),
            auth_policy.clone(),
        )?;

        if !no_sync {
            let mut env = crate::get_or_create_env(project_root.as_path())?;
            let lock = Lock::from_str(&wrapfs::read_to_string(
                project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME),
            )?)?;
            command_sync(
                &lock,
                project_root,
                &mut env,
                client,
                &provided_iris,
                runtime,
                auth_policy,
            )?;
        }
    }

    Ok(())
}
