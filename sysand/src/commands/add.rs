// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, str::FromStr, sync::Arc};

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use sysand_core::{
    add::do_add,
    auth::HTTPAuthentication,
    config::{
        Config, ConfigProject,
        local_fs::{CONFIG_FILE, add_project_source_to_config},
    },
    lock::Lock,
    project::{
        ProjectRead,
        local_src::LocalSrcProject,
        utils::{relativize_path, wrapfs},
    },
    resolve::{ResolutionOutcome, ResolveRead, standard::standard_resolver},
};

use crate::{
    CliError, DEFAULT_INDEX_URL,
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
    let iri = iri.as_ref();
    let mut current_project = current_project.ok_or(CliError::MissingProjectCurrentDir)?;
    let project_root = current_project.root_path().to_owned();

    #[allow(clippy::manual_map)] // For readability and compactness
    let source = if let Some(path) = source_opts.from_path {
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
    } else if let Some(url) = source_opts.from_url {
        let ResolutionOptions {
            index,
            default_index,
            no_index,
            include_std: _,
        } = resolution_opts.clone();

        let index_urls = if no_index {
            None
        } else {
            Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
        };
        let std_resolver = standard_resolver(
            None,
            None,
            Some(client.clone()),
            index_urls,
            runtime.clone(),
            auth_policy.clone(),
        );
        let outcome = std_resolver.resolve_read_raw(&url)?;
        let mut source = None;
        match outcome {
            ResolutionOutcome::Resolved(alternatives) => {
                for candidate in alternatives {
                    match candidate {
                        Ok(project) => {
                            source = project.sources().first().cloned();
                            if source.is_some() {
                                break;
                            }
                        }
                        Err(err) => {
                            log::debug!("skipping candidate project: {err}");
                        }
                    }
                }
            }
            ResolutionOutcome::UnsupportedIRIType(e) => bail!("unsupported URL: `{url}`\n{e}"),
            ResolutionOutcome::Unresolvable(e) => {
                bail!("failed to resolve URL: `{url}`: {e}")
            }
        }
        if source.is_none() {
            bail!("unable to find project at URL: `{url}`")
        }
        source
    } else if let Some(editable) = source_opts.as_editable {
        Some(sysand_core::lock::Source::Editable {
            editable: get_relative(editable, &project_root)?.as_str().into(),
        })
    } else if let Some(src_path) = source_opts.as_local_src {
        Some(sysand_core::lock::Source::LocalSrc {
            src_path: get_relative(src_path, &project_root)?.as_str().into(),
        })
    } else if let Some(kpar_path) = source_opts.as_local_kpar {
        Some(sysand_core::lock::Source::LocalKpar {
            kpar_path: get_relative(kpar_path, &project_root)?.as_str().into(),
        })
    } else if let Some(remote_src) = source_opts.as_remote_src {
        Some(sysand_core::lock::Source::RemoteSrc { remote_src })
    } else if let Some(remote_kpar) = source_opts.as_remote_kpar {
        Some(sysand_core::lock::Source::RemoteKpar {
            remote_kpar,
            remote_kpar_size: None,
        })
    } else if let Some(remote_git) = source_opts.as_remote_git {
        Some(sysand_core::lock::Source::RemoteGit { remote_git })
    } else {
        None
    };

    if let Some(source) = source {
        let config_path = config_file
            .map(Utf8PathBuf::from)
            .or((!no_config).then(|| project_root.join(CONFIG_FILE)));

        if let Some(path) = config_path {
            add_project_source_to_config(&path, iri, &source)?;
        } else {
            log::warn!("project source for `{iri}` not added to any config file");
        }

        config.projects.push(ConfigProject {
            identifiers: vec![iri.to_owned()],
            sources: vec![source],
        });
    }

    let provided_iris = if !resolution_opts.include_std {
        let sysml_std = crate::known_std_libs();
        if sysml_std.contains_key(iri) {
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
            let mut env = crate::get_or_create_env(&project_root)?;
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

/// `project_root` must be absolute. On Windows, its kind (DOS/UNC)
/// must match the kind of `current_dir()`
fn get_relative<P: Into<Utf8PathBuf> + AsRef<Utf8Path>>(
    src_path: P,
    project_root: &Utf8Path,
) -> Result<Utf8PathBuf> {
    let src_path = if src_path.as_ref().is_absolute() || wrapfs::current_dir()? != project_root {
        let path = relativize_path(wrapfs::canonicalize(src_path.as_ref())?, project_root)?;
        if path == "." {
            bail!("cannot add current project as usage of itself");
        }
        path
    } else {
        src_path.into()
    };
    Ok(src_path)
}
