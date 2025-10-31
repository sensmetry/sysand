// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use reqwest::blocking::Client;

use sysand_core::{add::do_add, lock::Lock, project::local_src::LocalSrcProject};

use crate::{CliError, cli::DependencyOptions, command_sync};

pub fn command_add(
    iri: String,
    versions_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    dependency_opts: DependencyOptions,
    current_project: Option<LocalSrcProject>,
    client: Client,
) -> Result<()> {
    let DependencyOptions {
        use_index,
        no_index,
        include_std,
    } = dependency_opts;
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    do_add(&mut current_project, iri, versions_constraint)?;

    let index_base_urls = if no_index { None } else { Some(use_index) };
    if !no_lock {
        let provided_iris = if !include_std {
            crate::known_std_libs()
        } else {
            std::collections::HashMap::default()
        };

        crate::commands::lock::command_lock(
            &project_root,
            client.clone(),
            index_base_urls,
            &provided_iris,
        )?;

        if !no_sync {
            let mut env = crate::get_or_create_env(project_root.as_path())?;
            let lock: Lock = toml::from_str(&std::fs::read_to_string(
                project_root.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME),
            )?)?;
            command_sync(lock, project_root, &mut env, client, &provided_iris)?;
        }
    }

    Ok(())
}
