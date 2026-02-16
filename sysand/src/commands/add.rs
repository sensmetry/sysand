// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::{Result, bail};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};

use sysand_core::{
    add::do_add,
    auth::HTTPAuthentication,
    config::{
        Config, ConfigProject,
        local_fs::{CONFIG_FILE, add_project_source_to_config},
    },
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
    mut config: Config,
    config_file: Option<String>,
    no_config: bool,
    current_project: Option<LocalSrcProject>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path();

    let config_path = config_file
        .map(Utf8PathBuf::from)
        .or((!no_config).then(|| project_root.join(CONFIG_FILE)));

    #[allow(clippy::manual_map)] // For readability and compactness
    let source = if let Some(path) = source_opts.as_local {
        let metadata = wrapfs::metadata(&path)?;
        if metadata.is_dir() {
            Some(sysand_core::lock::Source::LocalSrc {
                src_path: get_relative(path, &project_root)?.as_str().into(),
            })
        } else if metadata.is_file() {
            Some(sysand_core::lock::Source::LocalKpar {
                kpar_path: get_relative(path, &project_root)?.as_str().into(),
            })
        } else {
            bail!("path `{path}` is neither a directory nor a file");
        }
    } else if let Some(editable) = source_opts.as_editable {
        Some(sysand_core::lock::Source::Editable {
            editable: get_relative(editable, &project_root)?.as_str().into(),
        })
    } else if let Some(remote_src) = source_opts.as_url_src {
        Some(sysand_core::lock::Source::RemoteSrc { remote_src })
    } else if let Some(remote_kpar) = source_opts.as_url_kpar {
        Some(sysand_core::lock::Source::RemoteKpar {
            remote_kpar,
            remote_kpar_size: None,
        })
    } else {
        None
    };

    if let Some(source) = source {
        if let Some(path) = config_path {
            add_project_source_to_config(&path, &iri, &source)?;

            config.projects.push(ConfigProject {
                identifiers: vec![iri.as_ref().to_string()],
                sources: vec![source],
            });
        } else {
            bail!("must provide config file for specifying project source")
        }
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
            &config,
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

fn get_relative<P: Into<Utf8PathBuf>>(src_path: P, project_root: &Utf8Path) -> Result<Utf8PathBuf> {
    let src_path = if wrapfs::current_dir()? != project_root {
        let path = relativize(
            &Utf8Path::new(&src_path.into()).canonicalize_utf8()?,
            project_root,
        );
        if path.is_absolute() {
            bail!(
                "unable to find relative path from project root to `{}`",
                path,
            );
        }
        path
    } else {
        src_path.into()
    };
    Ok(src_path)
}

fn relativize(path: &Utf8Path, root: &Utf8Path) -> Utf8PathBuf {
    // If prefixes (e.g. C: vs D: on Windows) differ, no relative path is possible.
    if path.components().next() != root.components().next() {
        return path.to_path_buf();
    }

    let mut path_iter = path.components().peekable();
    let mut root_iter = root.components().peekable();

    while let (Some(p), Some(r)) = (path_iter.peek(), root_iter.peek()) {
        if p == r {
            path_iter.next();
            root_iter.next();
        } else {
            break;
        }
    }

    let mut result = Utf8PathBuf::new();

    for r in root_iter {
        if let Utf8Component::Normal(_) = r {
            result.push("..");
        }
    }

    for p in path_iter {
        result.push(p.as_str());
    }

    if result.as_str().is_empty() {
        result.push(".");
    }

    result
}
