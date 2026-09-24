// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, convert::Infallible, path::Path, sync::Arc};

use anyhow::{Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use sysand_core::{
    add::{AddError, do_add},
    auth::HTTPAuthentication,
    commands::{
        lock::{DEFAULT_LOCKFILE_NAME, LockOutcome, do_lock_local_editable},
        sync::SyncOutcome,
    },
    config::{
        Config, ConfigProject, OverrideSource,
        local_fs::{CONFIG_FILE, add_project_source_to_config},
    },
    context::ProjectContext,
    model::{IndexUsage, InterchangeProjectUsage, InterchangeProjectUsageRaw},
    project::{
        ProjectMut as _, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        utils::{Identifier, relativize_path, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead as _, standard::standard_resolver},
    utils::{ProvidedProjects, SP, format_err},
};

use crate::{
    CliError, DEFAULT_INDEX_URL,
    cli::{ProjectSourceOptions, ResolutionOptions, UsageLocator},
    commands::{
        lock::{create_resolver, resolve_lock},
        sync::command_sync,
    },
    style::GOOD,
};

// TODO: Collect common arguments
#[expect(clippy::fn_params_excessive_bools)]
pub fn command_add<Policy: HTTPAuthentication>(
    locator: UsageLocator,
    version_constraint: Option<String>,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    source_opts: Box<ProjectSourceOptions>,
    mut config: Config,
    config_file: Option<String>,
    no_config: bool,
    ctx: ProjectContext,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .clone()
        .ok_or(CliError::MissingProjectCurrentDir)?;

    // `identifier` is what config overrides, the lockfile and the standard
    // libraries know the project by. An index usage given without a
    // constraint is first added in `unconstrained_index`, see below.
    let (identifier, usage_raw, unconstrained_index) = match locator {
        UsageLocator::Iri(iri) => {
            let identifier = iri.to_string();
            let usage = InterchangeProjectUsageRaw::Resource {
                resource: iri.into_string(),
                version_constraint,
            };
            (identifier, usage, None)
        }
        UsageLocator::PublisherName { publisher, name } => {
            let identifier = Identifier::from_pub_name(&publisher, &name).into_string();
            match version_constraint {
                Some(version_constraint) => {
                    let usage = InterchangeProjectUsageRaw::Index(IndexUsage {
                        publisher,
                        name,
                        version_constraint,
                    });
                    (identifier, usage, None)
                }
                None if no_lock => bail!(
                    "an index usage needs a version constraint: pass one, or leave out\n\
                     `--no-lock` to use the version that locking chooses"
                ),
                None => {
                    let Some(info) = current_project.get_info()? else {
                        bail!(CliError::MissingProjectCurrentDir);
                    };
                    // Report a clash with an existing usage before resolving anything,
                    // as `do_add` would
                    if let Some(existing) = info.usage.iter().find(|u| {
                        Identifier::from_unvalidated_usage(u)
                            .is_some_and(|id| id.as_str() == identifier)
                    }) {
                        match existing {
                            InterchangeProjectUsageRaw::Index(IndexUsage {
                                publisher: p,
                                name: n,
                                version_constraint,
                            }) if *p == publisher && *n == name => {
                                log::warn!(
                                    "ignoring usage `{publisher}/{name}` without a version constraint,\n\
                                     {SP:>8} since it is already present with version constraint\n\
                                     {SP:>8} `{version_constraint}`",
                                );
                                return Ok(());
                            }
                            InterchangeProjectUsageRaw::Index(IndexUsage {
                                publisher: p,
                                name: n,
                                ..
                            }) => bail!(AddError::<Infallible>::IndexUsageSpelledDifferently {
                                existing: format!("{p}/{n}"),
                                new: format!("{publisher}/{name}"),
                            }),
                            other => bail!(AddError::<Infallible>::DuplicateIdentifier {
                                identifier,
                                existing: other.kind_with_article(),
                                new: "an index",
                            }),
                        }
                    }
                    // Resolved first as a resource usage of the same identifier and
                    // no constraint, which chooses the version the way an unconstrained
                    // usage always has (e.g. a prerelease from a source override)
                    let usage = InterchangeProjectUsageRaw::Resource {
                        resource: identifier.clone(),
                        version_constraint: None,
                    };
                    (identifier, usage, Some((publisher, name)))
                }
            }
        }
    };
    let iri = identifier.as_str();

    #[expect(clippy::manual_map)] // For readability and compactness
    let source = if let Some(path) = source_opts.from_path {
        let metadata = wrapfs::metadata(&path)?;
        if metadata.is_dir() {
            Some(OverrideSource::LocalSrc {
                src_path: get_relative(path, current_project.root_path(), &ctx.current_directory)?
                    .as_str()
                    .into(),
            })
        } else if metadata.is_file() {
            Some(OverrideSource::LocalKpar {
                kpar_path: get_relative(path, current_project.root_path(), &ctx.current_directory)?
                    .as_str()
                    .into(),
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
            strict_index_versions: _,
        } = resolution_opts.clone();

        let index_urls = if no_index {
            None
        } else {
            Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_owned()], default_index)?)
        };
        let std_resolver = standard_resolver(
            // TODO: why not use env here?
            None,
            Some(client.clone()),
            index_urls,
            runtime.clone(),
            auth_policy.clone(),
        )?;
        let resolve = ResolutionInfo::iri(url);
        let outcome = std_resolver.resolve_read(&resolve)?;
        let mut source = None;
        match outcome {
            ResolutionOutcome::Resolved(alternatives) => {
                for candidate in alternatives {
                    match candidate {
                        Ok(project) => {
                            source = Some(project.sources(&ctx)?[0].to_override());
                            break;
                        }
                        Err(err) => {
                            log::debug!("skipping candidate project: {}", format_err(err));
                        }
                    }
                }
            }
            ResolutionOutcome::UnsupportedUsageType { reason } => {
                bail!("unsupported project locator {resolve}: {reason}")
            }
            ResolutionOutcome::NotFound { reason } => {
                bail!("project not found at {resolve}: {reason}")
            }
            ResolutionOutcome::Unresolvable { reason } => {
                bail!("{resolve} is not resolvable: {reason}")
            }
        }
        if source.is_none() {
            bail!("unable to find project {resolve}")
        }
        source
    } else if let Some(editable) = source_opts.as_editable {
        Some(OverrideSource::Editable {
            editable: get_relative(
                editable,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(src_path) = source_opts.as_local_src {
        Some(OverrideSource::LocalSrc {
            src_path: get_relative(
                src_path,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(kpar_path) = source_opts.as_local_kpar {
        Some(OverrideSource::LocalKpar {
            kpar_path: get_relative(
                kpar_path,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(remote_src) = source_opts.as_remote_src {
        Some(OverrideSource::RemoteSrc {
            remote_src: remote_src.into_string(),
        })
    } else if let Some(remote_kpar) = source_opts.as_remote_kpar {
        // TODO: maybe also allow giving IndexKpar (does it make sense?)
        Some(OverrideSource::RemoteKpar {
            remote_kpar: remote_kpar.into_string(),
        })
    } else if let Some(remote_git) = source_opts.as_remote_git {
        Some(OverrideSource::RemoteGit {
            remote_git: remote_git.into_string(),
        })
    } else {
        None
    };

    if let Some(source) = source {
        let config_path = config_file
            .map(Utf8PathBuf::from)
            .or_else(|| (!no_config).then(|| current_project.root_path().join(CONFIG_FILE)));

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

    if no_lock {
        do_add(&mut current_project, &usage_raw)?;
        Ok(())
    } else {
        let info_path = current_project.info_path();
        let info_backup = wrapfs::read_to_string(&info_path)?;
        if unconstrained_index.is_some() {
            // Not `do_add`, so that only the final usage is reported as added
            let mut info = current_project
                .get_info()?
                .ok_or(CliError::MissingProjectCurrentDir)?;
            info.usage.push(usage_raw);
            current_project.put_info(&info, true)?;
        } else {
            let added = do_add(&mut current_project, &usage_raw)?;
            if !added {
                return Ok(());
            }
        }

        if let Some((publisher, name)) = unconstrained_index {
            let result = constrain_to_locked_version(
                &mut current_project,
                iri,
                publisher,
                name,
                &resolution_opts,
                &config,
                &client,
                &runtime,
                &auth_policy,
                &ctx,
            );
            if let Err(e) = result {
                wrapfs::write(&info_path, info_backup)?;
                return Err(e);
            }
        }

        let provided_iris = if resolution_opts.include_std {
            HashMap::default()
        } else {
            let sysml_std = crate::known_std_libs();
            if sysml_std.contains_key(iri) {
                log::info!(
                    "{GOOD}note{GOOD:#}: SysMLv2/KerML standard libraries will not be installed during sync"
                );
                // Can't skip either locking or syncing, since lockfile needs to add the usage,
                // and std version being added may affect version resolution of other packages
                // in the dependency graph (e.g. older versions used older std, newer use some
                // newer version)
            }
            sysml_std
        };

        let alias_iris = if let Some(w) = &ctx.current_workspace {
            w.projects()
                .iter()
                .find(|p| Path::new(&p.path) == current_project.root_path())
                .map(|p| p.iris.clone())
        } else {
            None
        };

        match resolve_deps(
            no_sync,
            no_prune,
            resolution_opts,
            &config,
            client,
            runtime,
            auth_policy,
            current_project.root_path(),
            alias_iris,
            provided_iris,
            ctx,
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Restore old info
                wrapfs::write(&info_path, info_backup)?;
                Err(e)
            }
        }
    }
}

/// Replace the usage of `identifier`, a resource usage with no constraint, by
/// the index usage of `publisher`/`name` constrained to `^` the version a
/// lock chooses for it, as `cargo add` does
fn constrain_to_locked_version<Policy: HTTPAuthentication>(
    project: &mut LocalSrcProject,
    identifier: &str,
    publisher: String,
    name: String,
    resolution_opts: &ResolutionOptions,
    config: &Config,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
    auth_policy: &Arc<Policy>,
    ctx: &ProjectContext,
) -> Result<()> {
    let lock = resolve_lock(
        ".",
        resolution_opts.clone(),
        config,
        project.root_path(),
        ProvidedProjects::default(),
        client.clone(),
        runtime.clone(),
        auth_policy.clone(),
        ctx,
    )?;
    let Some(locked) = lock
        .projects
        .iter()
        .find(|p| p.identifiers.iter().any(|id| id == identifier))
    else {
        bail!("`{publisher}/{name}` is missing from the lock");
    };
    let version_constraint = format!("^{}", locked.version);
    let mut info = project
        .get_info()?
        .ok_or(CliError::MissingProjectCurrentDir)?;
    info.usage.retain(|u| {
        !matches!(u, InterchangeProjectUsageRaw::Resource { resource, .. } if resource == identifier)
    });
    project.put_info(&info, true)?;
    do_add(
        project,
        &InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher,
            name,
            version_constraint,
        }),
    )?;
    Ok(())
}

pub enum ExpAddArgs {
    Dir { dir: Utf8PathBuf },
    KparPath { kpar_path: Utf8PathBuf },
}

// TODO: Collect common arguments
pub fn exp_command_add<Policy: HTTPAuthentication>(
    add: ExpAddArgs,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    config: Config,
    ctx: ProjectContext,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let mut current_project = ctx
        .current_project
        .clone()
        .ok_or(CliError::MissingProjectCurrentDir)?;

    let usage = match add {
        ExpAddArgs::Dir { dir } => {
            let abs_path = wrapfs::canonicalize(dir)?;
            let relative = relativize_path(&abs_path, current_project.root_path())?;
            let project = LocalSrcProject::new_access(abs_path, None);
            let info = project
                .get_info()?
                .ok_or_else(|| CliError::MissingProject(project.root_path().to_string()))?;
            let publisher = info.publisher.ok_or_else(|| {
                CliError::MissingPublisherForUsage(project.root_path().to_string())
            })?;
            InterchangeProjectUsage::Directory {
                dir: relative,
                publisher,
                name: info.name,
            }
        }
        ExpAddArgs::KparPath { kpar_path } => {
            let abs_path = wrapfs::canonicalize(kpar_path)?;
            let relative = relativize_path(&abs_path, current_project.root_path())?;
            let project = LocalKParProject::new_access(abs_path.clone(), KparInnerPath::Root, None);
            let info = project
                .get_info()?
                .ok_or_else(|| CliError::MissingProject(abs_path.to_string()))?;
            let publisher = info
                .publisher
                .ok_or_else(|| CliError::MissingPublisherForUsage(abs_path.to_string()))?;
            InterchangeProjectUsage::KparPath {
                kpar_path: relative,
                publisher,
                name: info.name,
            }
        }
    };

    if no_lock {
        do_add(&mut current_project, &usage.into())?;
        Ok(())
    } else {
        let info_path = current_project.info_path();
        let info_backup = wrapfs::read_to_string(&info_path)?;
        let added = do_add(&mut current_project, &usage.into())?;
        if !added {
            return Ok(());
        }

        let provided_iris = if resolution_opts.include_std {
            HashMap::default()
        } else {
            // Don't warn; std libs are all `https://`, so they can't match this usage
            crate::known_std_libs()
        };

        let alias_iris = if let Some(w) = &ctx.current_workspace {
            w.projects()
                .iter()
                .find(|p| Path::new(&p.path) == current_project.root_path())
                .map(|p| p.iris.clone())
        } else {
            None
        };

        match resolve_deps(
            no_sync,
            no_prune,
            resolution_opts,
            &config,
            client,
            runtime,
            auth_policy,
            current_project.root_path(),
            alias_iris,
            provided_iris,
            ctx,
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Restore old info
                wrapfs::write(&info_path, info_backup)?;
                Err(e)
            }
        }
    }
}

pub fn resolve_deps<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    project_root: P,
    project_identifiers: Option<Vec<Iri<String>>>,
    provided_iris: ProvidedProjects,
    ctx: ProjectContext,
) -> Result<(), anyhow::Error> {
    let solve_options = resolution_opts.solve_options();
    let resolver = create_resolver(
        resolution_opts,
        config,
        &project_root,
        &ctx,
        provided_iris.clone(),
        client.clone(),
        runtime.clone(),
        auth_policy.clone(),
    )?;
    // FIXME: use project path relative to and under the workspace root.
    let LockOutcome { lock, .. } = do_lock_local_editable(
        ".",
        &project_root,
        project_identifiers,
        &provided_iris,
        resolver,
        solve_options,
        &ctx,
    )?;
    let lock = lock.canonicalize();
    wrapfs::write(
        project_root.as_ref().join(DEFAULT_LOCKFILE_NAME),
        lock.to_string(),
    )?;
    if !no_sync {
        let mut env = crate::get_or_create_env(
            ctx.env,
            ctx.current_workspace.as_ref(),
            ctx.current_project.as_ref(),
            ctx.current_directory,
        )?;
        command_sync(
            &lock,
            project_root,
            &mut env,
            client,
            &provided_iris,
            runtime,
            auth_policy,
            ctx.current_workspace.as_ref(),
            no_prune,
            &mut SyncOutcome::default(),
        )?;
    }
    Ok(())
}

/// `project_root` must be absolute. On Windows, its kind (DOS/UNC)
/// must match the kind of `current_dir()`
fn get_relative<P: Into<Utf8PathBuf> + AsRef<Utf8Path>>(
    src_path: P,
    project_root: &Utf8Path,
    cwd: &Utf8Path,
) -> Result<Utf8PathBuf> {
    let src_path = if src_path.as_ref().is_absolute() || cwd != project_root {
        let path = relativize_path(wrapfs::canonicalize(src_path.as_ref())?, project_root)?;
        if path == "." {
            bail!("cannot add current project as usage of itself");
        }
        path.into_string().into()
    } else {
        src_path.into()
    };
    Ok(src_path)
}
