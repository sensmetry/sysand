// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{assert_matches, collections::HashMap, convert::Infallible, path::Path, sync::Arc};

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use semver::VersionReq;
use sysand_core::{
    add::{IndexSpellingError, IndexUsageToAdd, do_add, index_usage_to_add, spell_index_usage},
    auth::HTTPAuthentication,
    commands::{
        lock::{LockOutcome, do_lock_local_editable, do_lock_local_editable_respelling},
        sync::SyncOutcome,
    },
    config::{
        Config, ConfigProject, OverrideSource,
        local_fs::{CONFIG_FILE, add_project_source_to_config},
    },
    context::ProjectContext,
    lock::{Lock, Lockfile},
    model::{
        IndexName, IndexPublisher, InterchangeProjectUsage, InterchangeProjectUsageRaw,
        ProjectName, ProjectPublisher,
    },
    project::{
        ProjectMut as _, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        utils::{Identifier, relativize_path, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead as _, standard::standard_resolver},
    utils::{ProvidedProjects, format_err},
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

/// Returns whether `.project.json` changed: `false` when the usage was
/// already present as given (or without a constraint, which keeps the
/// existing one). Locks and syncs either way, unless `no_lock`/`no_sync`
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
            let Some(info) = current_project.get_info()? else {
                bail!(CliError::MissingProjectCurrentDir);
            };
            let index_usage = |publisher, name, version_constraint: &VersionReq| {
                InterchangeProjectUsageRaw::Index {
                    publisher,
                    name,
                    version_constraint: version_constraint.to_string(),
                }
            };
            // Refused, already present or settled by what is declared, before
            // anything is resolved or looked up
            match index_usage_to_add::<Infallible>(
                &info.usage,
                publisher,
                name,
                version_constraint,
            )? {
                IndexUsageToAdd::AlreadyPresent => UsageToAdd::AlreadyPresent,
                IndexUsageToAdd::Ready(usage) => UsageToAdd::Ready(usage),
                IndexUsageToAdd::New {
                    publisher,
                    name,
                    version_constraint,
                    normalized,
                } if no_lock => {
                    let Some(version_constraint) = version_constraint else {
                        bail!(
                            "an index usage needs a version constraint: pass one, or leave out\n\
                             `--no-lock` to use the version that locking chooses"
                        );
                    };
                    // Without locking, the spelling can only be checked against,
                    // or recovered from, what is installed
                    let (publisher, name) =
                        spell_index_usage(ctx.env.as_ref(), &publisher, &name, normalized)
                            .map_err(|err| match err {
                                IndexSpellingError::NotInstalled { .. } => anyhow!(
                                    "{err}\n{USAGE}hint:{USAGE:#} leave out `--no-lock` to look \
                                     the project up in the indexes"
                                ),
                                err => err.into(),
                            })?;
                    UsageToAdd::Ready(index_usage(
                        publisher.into_string(),
                        name.into_string(),
                        &version_constraint,
                    ))
                }
                IndexUsageToAdd::New {
                    publisher,
                    name,
                    version_constraint: Some(version_constraint),
                    normalized: false,
                } => {
                    // Locking checks the spelling
                    UsageToAdd::Ready(index_usage(
                        publisher.into_string(),
                        name.into_string(),
                        &version_constraint,
                    ))
                }
                IndexUsageToAdd::New {
                    publisher,
                    name,
                    version_constraint,
                    normalized,
                } => {
                    // The spelling or the version constraint is settled from
                    // the lock, see `settle_from_lock`
                    UsageToAdd::PendingIndex(PendingIndexUsage {
                        recover_spelling: normalized,
                        publisher,
                        name,
                        version_constraint,
                    })
                }
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
            let (publisher, name) =
                parse_project_spelling(publisher, info.name, project.root_path().as_str())?;
            let usage = InterchangeProjectUsage::Directory {
                dir: relative,
                publisher,
                name,
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
            let (publisher, name) =
                parse_project_spelling(publisher, info.name, abs_path.as_str())?;
            let usage = InterchangeProjectUsage::KparPath {
                kpar_path: relative,
                publisher,
                name,
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
        return match usage {
            UsageToAdd::Ready(usage) => Ok(do_add(&mut current_project, &usage)?),
            UsageToAdd::AlreadyPresent => Ok(false),
            UsageToAdd::PendingIndex(_) => {
                unreachable!("without locking, an index usage is settled from the environment")
            }
        };
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
    // Even when nothing is added, lock and sync, since the environment may
    // be missing or stale
    let (added, pending) = match usage {
        UsageToAdd::Ready(usage) => (do_add(&mut current_project, &usage)?, None),
        UsageToAdd::AlreadyPresent => (false, None),
        // Added as given, and settled once locked
        UsageToAdd::PendingIndex(pending) => {
            let placeholder = pending.placeholder();
            (
                do_add(&mut current_project, &placeholder)?,
                Some((placeholder, pending)),
            )
        }
    };

    let alias_iris = if let Some(w) = &ctx.current_workspace {
        w.projects()
            .iter()
            .find(|p| Path::new(&p.path) == current_project.root_path())
            .map(|p| p.iris.clone())
    } else {
        None
    };

    let project_root = current_project.root_path().to_owned();
    let locked = (|| {
        let respelled = pending
            .as_ref()
            .and_then(|(_, pending)| pending.respelled());
        let lock = lock_project(
            resolver,
            &project_root,
            alias_iris,
            &provided_iris,
            &ctx,
            respelled.as_ref(),
        )?;
        if let Some((placeholder, pending)) = pending {
            settle_from_lock(&mut current_project, &lock, &placeholder, pending)?;
        }
        Ok::<_, anyhow::Error>(lock)
    })();
    let result = locked.and_then(|lock| {
        write_lock_and_sync(
            lock,
            no_sync,
            no_prune,
            client,
            runtime,
            auth_policy,
            &project_root,
            &provided_iris,
            ctx,
        )
    });
    match result {
        Ok(()) => Ok(added),
        Err(e) => {
            // Restore old info
            wrapfs::write(&info_path, info_backup)?;
            Err(e)
        }
    }
}

/// The usage `add` adds, the index usage it is yet to settle, or nothing to
/// add, as the usage is already present
enum UsageToAdd {
    Ready(InterchangeProjectUsageRaw),
    PendingIndex(PendingIndexUsage),
    AlreadyPresent,
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

/// The publisher and name of the project at `path`, which a directory or KPAR
/// usage of it spells
fn parse_project_spelling(
    publisher: String,
    name: String,
    path: &str,
) -> Result<(ProjectPublisher, ProjectName)> {
    let publisher = ProjectPublisher::parse(publisher).map_err(|(publisher, e)| {
        anyhow!("project `{path}` has an invalid publisher `{publisher}`: {e}")
    })?;
    let name = ProjectName::parse(name)
        .map_err(|(name, e)| anyhow!("project `{path}` has an invalid name `{name}`: {e}"))?;
    Ok((publisher, name))
}

/// An index usage that `add` settles from the lock (see [`settle_from_lock`])
struct PendingIndexUsage {
    publisher: IndexPublisher,
    name: IndexName,
    /// Taken from the lock when `None`
    version_constraint: Option<VersionReq>,
    /// Whether `publisher`/`name` is normalized, and the usage is to take
    /// the spelling of the project it locks to instead
    recover_spelling: bool,
}

impl PendingIndexUsage {
    /// The usage as given, added before locking: an unconstrained one
    /// accepts any release
    fn placeholder(&self) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::Index {
            publisher: self.publisher.as_str().to_owned(),
            name: self.name.as_str().to_owned(),
            version_constraint: self
                .version_constraint
                .as_ref()
                .map_or_else(|| "*".to_owned(), ToString::to_string),
        }
    }

    /// The identifier whose spelling the lock is not to check, when the
    /// spelling is to be taken from the lock instead
    fn respelled(&self) -> Option<Identifier> {
        self.recover_spelling
            .then(|| Identifier::from_index(&self.publisher, &self.name))
    }
}

/// Replace `placeholder`, the usage `pending` added as given, in `project`:
/// spelled as the project it locked to when it was normalized, and
/// constrained to `^` the locked version when it had no constraint, as
/// `cargo add` does. Nothing has to be resolved for it beforehand, since an
/// index usage resolves by identifier, whatever its spelling.
fn settle_from_lock(
    project: &mut LocalSrcProject,
    lock: &Lock,
    placeholder: &InterchangeProjectUsageRaw,
    pending: PendingIndexUsage,
) -> Result<()> {
    let PendingIndexUsage {
        publisher,
        name,
        version_constraint,
        recover_spelling,
    } = pending;
    let identifier = Identifier::from_index(&publisher, &name);
    // It was just locked
    let locked = lock
        .projects
        .iter()
        .find(|p| p.identifiers.iter().any(|id| id == identifier.as_str()))
        .expect("BUG: the usage locked is missing from the lock");
    let (publisher, name) = if recover_spelling {
        let Some(locked_publisher) = &locked.publisher else {
            bail!(
                "`{publisher}/{name}` locked to version {} of a project that declares no \
                 publisher, which an index usage cannot name",
                locked.version
            );
        };
        match (
            IndexPublisher::parse(locked_publisher.clone()),
            IndexName::parse(locked.name.clone()),
        ) {
            (Ok(publisher), Ok(name)) => (publisher, name),
            _ => bail!(
                "`{publisher}/{name}` locked to version {} of a project spelled \
                 `{locked_publisher}/{}`, which an index usage cannot spell",
                locked.version,
                locked.name
            ),
        }
    } else {
        (publisher, name)
    };
    let version_constraint =
        version_constraint.map_or_else(|| format!("^{}", locked.version), |vc| vc.to_string());
    let settled = InterchangeProjectUsageRaw::Index {
        publisher: publisher.into_string(),
        name: name.into_string(),
        version_constraint,
    };
    let settling = "Settled";
    let header = sysand_core::style::get_style_config().header;
    log::info!("{header}{settling:>12}{header:#} usage: {settled}");
    if settled == *placeholder {
        return Ok(());
    }
    let mut info = project
        .get_info()?
        .ok_or(CliError::MissingProjectCurrentDir)?;
    // FIXME: write once, after spelling is known
    for usage in &mut info.usage {
        if usage == placeholder {
            *usage = settled.clone();
        }
    }
    project.put_info(&info, true)?;
    Ok(())
}

/// Lock the project at `project_root` with `resolver` (see
/// [`create_resolver`]), without writing the lockfile. The spelling of its
/// typed usage of `respelled` is not checked, see
/// [`do_lock_local_editable_respelling`]
fn lock_project<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    resolver: CliResolver<Policy>,
    project_root: P,
    project_identifiers: Option<Vec<Iri<String>>>,
    provided_iris: &ProvidedProjects,
    ctx: &ProjectContext,
    respelled: Option<&Identifier>,
) -> Result<Lock> {
    // FIXME: use project path relative to and under the workspace root.
    let LockOutcome { lock, .. } = match respelled {
        None => do_lock_local_editable(
            ".",
            &project_root,
            project_identifiers,
            provided_iris,
            resolver,
            ctx,
        )?,
        Some(respelled) => do_lock_local_editable_respelling(
            ".",
            &project_root,
            project_identifiers,
            provided_iris,
            resolver,
            ctx,
            respelled,
        )?,
    };
    Ok(lock.canonicalize())
}

/// Write `lock` as the lockfile of the project at `project_root`, and sync
/// unless `no_sync`
#[expect(clippy::too_many_arguments)]
fn write_lock_and_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: Lock,
    no_sync: bool,
    no_prune: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    project_root: P,
    provided_iris: &ProvidedProjects,
    ctx: ProjectContext,
) -> Result<(), anyhow::Error> {
    let lockfile = Lockfile::new(&project_root, lock);
    lockfile.write()?;
    if !no_sync {
        let mut env = crate::get_or_create_env(
            ctx.env,
            ctx.current_workspace.as_ref(),
            ctx.current_project.as_ref(),
            ctx.current_directory,
        )?;
        command_sync(
            lockfile.lock(),
            project_root,
            &mut env,
            client,
            provided_iris,
            runtime,
            auth_policy,
            ctx.current_workspace.as_ref(),
            no_prune,
            &mut SyncOutcome::default(),
        )?;
    }
    Ok(())
}

/// Lock with `resolver` (see [`create_resolver`]), and sync unless `no_sync`
#[expect(clippy::too_many_arguments)]
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
    let lock = lock_project(
        resolver,
        &project_root,
        project_identifiers,
        &provided_iris,
        &ctx,
        None,
    )?;
    write_lock_and_sync(
        lock,
        no_sync,
        no_prune,
        client,
        runtime,
        auth_policy,
        project_root,
        &provided_iris,
        ctx,
    )
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
