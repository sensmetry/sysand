// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::CliError;
use sysand_core::resolve::standard::standard_resolver;

use anyhow::{Result, bail};
use fluent_uri::Iri;
use reqwest::blocking::Client;
use std::{env::current_dir, path::Path};
use sysand_core::{
    info::{do_info, do_info_project},
    model::InterchangeProjectInfoRaw,
    project::{local_kpar::LocalKParProject, local_src::LocalSrcProject},
};

pub fn pprint_interchange_project(info: InterchangeProjectInfoRaw) {
    println!("Name: {}", info.name);
    if let Some(description) = info.description {
        println!("Description: {}", description);
    }
    println!("Version: {}", info.version);
    if let Some(license) = info.license {
        println!("License: {}", license);
    }
    if let Some(website) = info.website {
        println!("Website: {}", website);
    }
    if !info.maintainer.is_empty() {
        println!("Maintainer(s): {}", info.maintainer.join(", "));
    }
    if !info.topic.is_empty() {
        println!("Topics: {}", info.topic.join(", "));
    }

    if info.usage.is_empty() {
        println!("No usages.");
    } else {
        for usage in info.usage {
            print!("    Usage: {}", usage.resource);
            if let Some(v) = usage.version_constraint {
                println!(" ({})", v);
            } else {
                println!();
            }
        }
    }
}

pub fn command_info_path<P: AsRef<Path>>(path: P) -> Result<()> {
    let project = if path.as_ref().is_file() {
        sysand_core::resolve::file::FileResolverProject::LocalKParProject(
            LocalKParProject::new_guess_root(path.as_ref())?,
        )
    } else if path.as_ref().is_dir() {
        sysand_core::resolve::file::FileResolverProject::LocalSrcProject(LocalSrcProject {
            project_path: path.as_ref().to_path_buf(),
        })
    } else {
        bail!(CliError::NoResolve(format!(
            "Unable to find interchange project at {}",
            path.as_ref().display()
        )));
    };

    match do_info_project(project) {
        Some((info, _)) => {
            pprint_interchange_project(info);

            Ok(())
        }
        None => bail!(CliError::NoResolve(format!(
            "Unable to find interchange project at {}",
            path.as_ref().display()
        ))),
    }
}

pub fn command_info_uri<S: AsRef<str>>(
    uri: Iri<String>,
    _normalise: bool,
    client: Client,
    index_base_urls: Option<Vec<S>>,
) -> Result<()> {
    let cwd = current_dir().ok();

    let local_env_path =
        std::path::Path::new(".").join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

    let combined_resolver = standard_resolver(
        cwd,
        if local_env_path.is_dir() {
            Some(local_env_path)
        } else {
            None
        },
        Some(client),
        index_base_urls
            .map(|xs| xs.iter().map(|x| url::Url::parse(x.as_ref())).collect())
            .transpose()?,
    );

    let mut found = false;

    for (info, _) in do_info(&uri, &combined_resolver)? {
        found = true;
        pprint_interchange_project(info);
    }

    if !found {
        // FIXME: The more precise error messages are ignored here. For example,
        // if a user provides a relative file URI (this is invalid since file
        // URIs have to be absolute), the error message will be saying that the
        // interchange project was not found without any hints that the provided
        // URI is invalid.
        bail!(CliError::NoResolve(format!(
            "Unable to find interchange project at {}",
            uri.as_str()
        )));
    }

    Ok(())
}
