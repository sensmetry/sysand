// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{assert_matches, collections::HashMap, convert::Infallible, path::Path, sync::Arc};

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use semver::{Version, VersionReq};
use sysand_core::{
    add::{AddError, IndexSpellingError, do_add, is_normalized_spelling, spell_index_usage},
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
    model::{InterchangeProjectUsage, InterchangeProjectUsageRaw, check_index_usage_spelling},
    project::{
        ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        utils::{Identifier, relativize_path, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead, standard::standard_resolver},
    solve::pubgrub::DEFAULT_INDEX_CONSTRAINT,
    utils::{ProvidedProjects, SP, format_err},
};

use crate::{
    CliError,
    cli::{AddProjectLocatorArgs, ProjectSourceOptions, ResolutionOptions},
    commands::{
        lock::{CliResolver, create_resolver},
        sync::command_sync,
    },
    default_index_location, iri_or_path_to_iri,
    style::{GOOD, USAGE},
};

/// Returns whether a usage was added: `false` when the usage was already
/// present, and the call was merged into (or ignored for) it
// TODO: Collect common arguments
#[expect(clippy::fn_params_excessive_bools)]
pub fn command_add<Policy: HTTPAuthentication>(
    locator: AddProjectLocatorArgs,
    // iri: Iri<String>,
    version_constraint: Option<VersionReq>,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    source_overrides: Box<ProjectSourceOptions>,
    mut config: Config,
    config_file: Option<Utf8PathBuf>,
    no_config: bool,
    ctx: ProjectContext,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<bool> {
    let mut current_project = ctx
        .current_project
        .clone()
        .ok_or(CliError::MissingProjectCurrentDir)?;

    let usage = match locator {
        AddProjectLocatorArgs {
            identifier: Some((publisher, name)),
            dir: None,
            kpar_path: None,
            iri: None,
            iri_path: None,
        } => {
            // Source overrides are only for resource usages
            assert_no_overrides(&source_overrides);
            let (publisher, name) = (publisher.into_string(), name.into_string());
            check_index_usage_spelling(&publisher, &name)?;
            let identifier = Identifier::from_pub_name(&publisher, &name).into_string();
            let Some(info) = current_project.get_info()? else {
                bail!(CliError::MissingProjectCurrentDir);
            };
            let existing = info.usage.iter().find(|u| {
                Identifier::from_unvalidated_usage(u).is_some_and(|id| id.as_str() == identifier)
            });
            // A normalized spelling names the project by its identifier only,
            // so an index usage of it already declared says how it is spelled
            let (publisher, name) = match existing {
                Some(InterchangeProjectUsageRaw::Index {
                    publisher: p,
                    name: n,
                    ..
                }) if is_normalized_spelling(&publisher, &name) => (p.clone(), n.clone()),
                _ => (publisher, name),
            };
            // Report a clash with a usage of another kind before resolving
            // or looking up anything, as `do_add` would
            if let Some(other) = existing
                && !matches!(other, InterchangeProjectUsageRaw::Index { .. })
            {
                bail!(AddError::<Infallible>::DuplicateIdentifier {
                    identifier,
                    existing: other.kind_with_article(),
                    new: "an index",
                });
            }
            let index_usage = |publisher, name, version_constraint: &VersionReq| {
                InterchangeProjectUsageRaw::Index {
                    publisher,
                    name,
                    version_constraint: version_constraint.to_string(),
                }
            };
            if no_lock {
                let Some(version_constraint) = version_constraint else {
                    bail!(
                        "an index usage needs a version constraint: pass one, or leave out\n\
                         `--no-lock` to use the version that locking chooses"
                    );
                };
                // Without locking, the spelling can only be checked against, or
                // recovered from, what is installed
                let (publisher, name) =
                    spell_index_usage(ctx.env.as_ref(), &publisher, &name, &version_constraint)
                        .map_err(|err| match err {
                            IndexSpellingError::NotInstalled { .. } => anyhow!(
                                "{err}\n{USAGE}hint:{USAGE:#} leave out `--no-lock` to look the \
                         project up in the indexes"
                            ),
                            err => err.into(),
                        })?;
                UsageToAdd::Ready(index_usage(publisher, name, &version_constraint))
            } else if let Some(InterchangeProjectUsageRaw::Index {
                publisher: p,
                name: n,
                version_constraint: existing_constraint,
            }) = existing
            {
                match version_constraint {
                    None if *p == publisher && *n == name => {
                        log::warn!(
                            "ignoring usage `{publisher}/{name}` without a version constraint,\n\
                             {SP:>8} since it is already present with version constraint\n\
                             {SP:>8} `{existing_constraint}`",
                        );
                        return Ok(false);
                    }
                    None => bail!(AddError::<Infallible>::TypedUsageSpelledDifferently {
                        kind: "an index",
                        existing: format!("{p}/{n}"),
                        new: format!("{publisher}/{name}"),
                    }),
                    // `do_add` merges the constraints (or reports a different
                    // spelling), and locking checks the spelling
                    Some(version_constraint) => {
                        UsageToAdd::Ready(index_usage(publisher, name, &version_constraint))
                    }
                }
            } else if let Some(version_constraint) = &version_constraint
                && !is_normalized_spelling(&publisher, &name)
            {
                // Locking checks the spelling
                UsageToAdd::Ready(index_usage(publisher, name, version_constraint))
            } else {
                // The spelling or the version constraint is settled by
                // resolving it, see `settle_index_usage`
                UsageToAdd::PendingIndex(PendingIndexUsage {
                    recover_spelling: is_normalized_spelling(&publisher, &name),
                    publisher,
                    name,
                    version_constraint,
                })
            }
        }
        AddProjectLocatorArgs {
            identifier: None,
            dir: Some(dir),
            kpar_path: None,
            iri: None,
            iri_path: None,
        } => {
            assert_no_overrides(&source_overrides);
            assert_eq!(version_constraint, None);
            let abs_path = wrapfs::canonicalize(dir)?;
            let relative = relativize_path(&abs_path, current_project.root_path())?;
            let project = LocalSrcProject::new_access(abs_path, None);
            let info = project
                .get_info()?
                .ok_or_else(|| CliError::MissingProject(project.root_path().to_string()))?;
            let publisher = info.publisher.ok_or_else(|| {
                CliError::MissingPublisherForUsage(project.root_path().to_string())
            })?;
            let usage = InterchangeProjectUsage::Directory {
                dir: relative,
                publisher,
                name: info.name,
            };
            UsageToAdd::Ready(usage.into())
        }
        AddProjectLocatorArgs {
            identifier: None,
            dir: None,
            kpar_path: Some(kpar_path),
            iri: None,
            iri_path: None,
        } => {
            assert_no_overrides(&source_overrides);
            assert_eq!(version_constraint, None);
            let abs_path = wrapfs::canonicalize(kpar_path)?;
            let relative = relativize_path(&abs_path, current_project.root_path())?;
            let project = LocalKParProject::new_access(abs_path.clone(), KparInnerPath::Root, None);
            let info = project
                .get_info()?
                .ok_or_else(|| CliError::MissingProject(abs_path.to_string()))?;
            let publisher = info
                .publisher
                .ok_or_else(|| CliError::MissingPublisherForUsage(abs_path.to_string()))?;
            let usage = InterchangeProjectUsage::KparPath {
                kpar_path: relative,
                publisher,
                name: info.name,
            };
            UsageToAdd::Ready(usage.into())
        }
        AddProjectLocatorArgs {
            identifier: None,
            dir: None,
            kpar_path: None,
            iri,
            iri_path,
        } => {
            let iri = iri_or_path_to_iri(iri, iri_path)?;
            process_overrides(
                &resolution_opts,
                source_overrides,
                &mut config,
                config_file,
                no_config,
                &ctx,
                &client,
                &runtime,
                &auth_policy,
                &current_project,
                &iri,
            )?;
            let usage = InterchangeProjectUsage::Resource {
                resource: iri,
                version_constraint,
            };
            UsageToAdd::Ready(usage.into())
        }
        _ => unreachable!(),
    };

    if no_lock {
        let UsageToAdd::Ready(usage) = usage else {
            unreachable!("without locking, an index usage is settled from the environment");
        };
        return Ok(do_add(&mut current_project, &usage)?);
    }

    let info_path = current_project.info_path();
    let info_backup = wrapfs::read_to_string(&info_path)?;

    let provided_iris = if resolution_opts.include_std {
        HashMap::default()
    } else {
        let sysml_std = crate::known_std_libs();
        if let UsageToAdd::Ready(InterchangeProjectUsageRaw::Resource { resource, .. }) = &usage
            && sysml_std.contains_key(resource.as_str())
        {
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

    // One resolver for settling the usage and for locking, so that what is
    // fetched to settle it is reused by the lock
    let resolver = create_resolver(
        resolution_opts,
        &config,
        current_project.root_path(),
        &ctx,
        provided_iris.clone(),
        client.clone(),
        runtime.clone(),
        auth_policy.clone(),
    )?;
    let usage = match usage {
        UsageToAdd::Ready(usage) => usage,
        UsageToAdd::PendingIndex(pending) => settle_index_usage(&resolver, pending)?,
    };
    if !do_add(&mut current_project, &usage)? {
        return Ok(false);
    }

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
        resolver,
        client,
        runtime,
        auth_policy,
        current_project.root_path(),
        alias_iris,
        provided_iris,
        ctx,
    ) {
        Ok(()) => Ok(true),
        Err(e) => {
            // Restore old info
            wrapfs::write(&info_path, info_backup)?;
            Err(e)
        }
    }
}

/// The usage `add` adds, or the index usage it is yet to settle
enum UsageToAdd {
    Ready(InterchangeProjectUsageRaw),
    PendingIndex(PendingIndexUsage),
}

fn process_overrides<Policy: HTTPAuthentication>(
    resolution_opts: &ResolutionOptions,
    source_overrides: Box<ProjectSourceOptions>,
    config: &mut Config,
    config_file: Option<Utf8PathBuf>,
    no_config: bool,
    ctx: &ProjectContext,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
    auth_policy: &Arc<Policy>,
    current_project: &LocalSrcProject,
    iri: &Iri<String>,
) -> Result<(), anyhow::Error> {
    #[expect(clippy::manual_map)] // For readability and compactness
    let source = if let Some(path) = source_overrides.from_path {
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
    } else if let Some(url) = source_overrides.from_url {
        let ResolutionOptions {
            index,
            default_index,
            no_index,
            include_std: _,
        } = resolution_opts.clone();

        let index_urls = if no_index {
            None
        } else {
            Some(config.index_urls(index, vec![default_index_location()], default_index))
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
                            source = Some(project.sources(ctx)?[0].to_override());
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
    } else if let Some(editable) = source_overrides.as_editable {
        Some(OverrideSource::Editable {
            editable: get_relative(
                editable,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(src_path) = source_overrides.as_local_src {
        Some(OverrideSource::LocalSrc {
            src_path: get_relative(
                src_path,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(kpar_path) = source_overrides.as_local_kpar {
        Some(OverrideSource::LocalKpar {
            kpar_path: get_relative(
                kpar_path,
                current_project.root_path(),
                &ctx.current_directory,
            )?
            .as_str()
            .into(),
        })
    } else if let Some(remote_src) = source_overrides.as_remote_src {
        Some(OverrideSource::RemoteSrc { remote_src })
    } else if let Some(remote_kpar) = source_overrides.as_remote_kpar {
        // TODO: maybe also allow giving IndexKpar (does it make sense?)
        Some(OverrideSource::RemoteKpar { remote_kpar })
    } else if let Some(remote_git) = source_overrides.as_remote_git {
        Some(OverrideSource::RemoteGit { remote_git })
    } else {
        None
    };
    let _: () = if let Some(source) = source {
        let config_path = config_file
            .or_else(|| (!no_config).then(|| current_project.root_path().join(CONFIG_FILE)));

        if let Some(path) = config_path {
            add_project_source_to_config(&path, iri.borrow(), &source)?;
        } else {
            log::warn!("project source for `{iri}` not added to any config file");
        }

        config.projects.push(ConfigProject {
            identifiers: vec![iri.clone()],
            sources: vec![source],
        });
    };
    Ok(())
}

/// An index usage that `add` settles by resolving it (see
/// [`settle_index_usage`])
struct PendingIndexUsage {
    publisher: String,
    name: String,
    /// Taken from the version resolved when `None`
    version_constraint: Option<VersionReq>,
    /// Whether `publisher`/`name` is normalized, and the usage is to take
    /// the spelling of the project resolved instead
    recover_spelling: bool,
}

/// The index usage `pending` names, settled by resolving it. Index usages
/// resolve by identifier, whatever their spelling, so this finds the
/// project's versions either way. Of those `pending` accepts (or, without a
/// constraint, of the releases), the highest is taken: the usage's version
/// constraint, when it has none, is `^` that version, as `cargo add` does,
/// and its spelling, when it is normalized, is that version's.
///
/// The rest of the dependency graph is not considered: if another project
/// requires an older version (e.g. `^1` while the highest is 2.0.0), the
/// usage written does not lock, and `add` fails, even though a lower
/// version would work.
fn settle_index_usage<R: ResolveRead>(
    resolver: &R,
    pending: PendingIndexUsage,
) -> Result<InterchangeProjectUsageRaw> {
    let PendingIndexUsage {
        publisher,
        name,
        version_constraint,
        recover_spelling,
    } = pending;
    let constraint = version_constraint
        .clone()
        .unwrap_or(DEFAULT_INDEX_CONSTRAINT);
    let resolve = ResolutionInfo::new(
        InterchangeProjectUsage::Index {
            publisher: publisher.clone(),
            name: name.clone(),
            version_constraint: constraint.clone(),
        },
        None,
    );
    let candidates = match resolver.resolve_read(&resolve)? {
        ResolutionOutcome::Resolved(candidates) => candidates,
        ResolutionOutcome::NotFound { reason }
        | ResolutionOutcome::Unresolvable { reason }
        | ResolutionOutcome::UnsupportedUsageType { reason } => {
            bail!("cannot find `{publisher}/{name}`: {reason}")
        }
    };
    let mut highest: Option<(Version, Option<String>, String)> = None;
    for candidate in candidates {
        let info = match candidate.map(|project| project.get_info()) {
            Ok(Ok(Some(info))) => info,
            Ok(Ok(None)) => continue,
            Ok(Err(err)) => {
                log::debug!("skipping candidate for {resolve}: {}", format_err(err));
                continue;
            }
            Err(err) => {
                log::debug!("skipping candidate for {resolve}: {}", format_err(err));
                continue;
            }
        };
        let Ok(version) = Version::parse(&info.version) else {
            continue;
        };
        if constraint.matches(&version)
            && highest
                .as_ref()
                .is_none_or(|(highest, ..)| version > *highest)
        {
            highest = Some((version, info.publisher, info.name));
        }
    }
    let Some((version, resolved_publisher, resolved_name)) = highest else {
        bail!("no version of `{publisher}/{name}` matching `{constraint}` is found");
    };
    let (publisher, name) = if recover_spelling {
        let Some(resolved_publisher) = resolved_publisher else {
            bail!(
                "`{publisher}/{name}` resolved to version {version} of a project that declares \
                 no publisher, which an index usage cannot name"
            );
        };
        (resolved_publisher, resolved_name)
    } else {
        (publisher, name)
    };
    let version_constraint =
        version_constraint.map_or_else(|| format!("^{version}"), |vc| vc.to_string());
    Ok(InterchangeProjectUsageRaw::Index {
        publisher,
        name,
        version_constraint,
    })
}

/// Lock with `resolver` (see [`create_resolver`]), and sync unless `no_sync`
pub fn resolve_deps<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    no_sync: bool,
    no_prune: bool,
    resolver: CliResolver<Policy>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    project_root: P,
    project_identifiers: Option<Vec<Iri<String>>>,
    provided_iris: ProvidedProjects,
    ctx: ProjectContext,
) -> Result<(), anyhow::Error> {
    // FIXME: use project path relative to and under the workspace root.
    let LockOutcome { lock, .. } = do_lock_local_editable(
        ".",
        &project_root,
        project_identifiers,
        &provided_iris,
        resolver,
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

fn assert_no_overrides(source_overrides: &ProjectSourceOptions) {
    assert_matches!(
        source_overrides,
        ProjectSourceOptions {
            from_path: None,
            from_url: None,
            as_editable: None,
            as_local_src: None,
            as_local_kpar: None,
            as_remote_src: None,
            as_remote_kpar: None,
            as_remote_git: None
        }
    );
}
