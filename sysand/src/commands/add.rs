// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{assert_matches, sync::Arc};
use sysand_core::paths::relativize_path;

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};

use fluent_uri::Iri;
use semver::VersionReq;
use sysand_core::{
    add::{
        AddUsage, IndexSpellingError, IndexUsageToAdd, SettleUsage, index_usage_to_add,
        spell_index_usage,
    },
    auth::HTTPAuthentication,
    commands::sync::SyncOutcome,
    config::{
        Config, ConfigProject, OverrideSource,
        local_fs::{CONFIG_FILE, add_project_source_to_config},
    },
    context::{BackupError, ManifestBackup, ProjectContext},
    lock::{Lock, Lockfile},
    manifest::common::normalized_identity,
    model::{
        IndexName, IndexPublisher, InterchangeProjectUsage, InterchangeProjectUsageRaw,
        ProjectName, ProjectPublisher,
    },
    project::{
        ProjectDefinition, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        utils::{Identifier, wrapfs},
    },
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead as _, standard::standard_resolver},
    utils::{ProvidedProjects, format_err},
};

use crate::{
    CliError,
    cli::{AddProjectLocatorArgs, ProjectSourceOptions, ResolutionOptions},
    commands::{
        lock::{CliResolver, LockFailure, create_resolver, lock_scope},
        sync::sync_context,
    },
    iri_or_path_to_iri,
    style::{GOOD, USAGE},
};

/// Returns whether the manifest changed: `false` when the usage was
/// already present as given (or without a constraint, which keeps the
/// existing one). Locks and syncs either way, unless `no_lock`/`no_sync`.
// TODO: Collect common arguments
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "mirrors the independent boolean CLI flags"
)]
pub fn command_add<Policy: HTTPAuthentication>(
    locator: AddProjectLocatorArgs,
    version_constraint: Option<VersionReq>,
    no_lock: bool,
    no_sync: bool,
    no_prune: bool,
    resolution_opts: ResolutionOptions,
    source_overrides: Box<ProjectSourceOptions>,
    mut config: Config,
    config_file: Option<Utf8PathBuf>,
    no_config: bool,
    mut ctx: ProjectContext,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<bool, LockFailure<Policy>> {
    let project_root = ctx
        .project_root()
        .ok_or(CliError::MissingProjectCurrentDir)?;

    // The source override of a resource usage, recorded only once the
    // usage is in the manifest (see `record_override`)
    let mut pending_override = None;
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
            let Some(current) = ctx.current_definition() else {
                return Err(anyhow!(CliError::MissingProjectCurrentDir).into());
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
            match index_usage_to_add::<!>(&current.usages(), publisher, name, version_constraint)? {
                IndexUsageToAdd::AlreadyPresent => UsageToAdd::AlreadyPresent,
                IndexUsageToAdd::Ready(usage) => UsageToAdd::Ready(usage),
                IndexUsageToAdd::New {
                    publisher,
                    name,
                    version_constraint,
                    normalized: _,
                } if no_lock => {
                    let Some(version_constraint) = version_constraint else {
                        return Err(anyhow!(
                            "an index usage needs a version constraint: pass one, or leave out\n\
                             `--no-lock` to use the version that locking chooses"
                        )
                        .into());
                    };
                    // Without locking, the spelling can only be checked against,
                    // or recovered from, what is installed
                    let (publisher, name) = spell_index_usage(ctx.env.as_ref(), &publisher, &name)
                        .map_err(|err| match err {
                            IndexSpellingError::NotInstalled {
                                usage: _,
                                normalized: _,
                            } => anyhow!(
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
            let relative = relativize_path(&abs_path, &project_root)?;
            let project = ctx.project_at_path(abs_path, None)?;
            let definition = project
                .get_project()?
                .ok_or_else(|| CliError::MissingProject(project.root_path().to_string()))?;
            let publisher = definition.publisher().ok_or_else(|| {
                CliError::MissingPublisherForUsage(project.root_path().to_string())
            })?;
            let (publisher, name) = parse_project_spelling(
                publisher.to_owned(),
                definition.name().to_owned(),
                project.root_path().as_str(),
            )?;
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
            let relative = relativize_path(&abs_path, &project_root)?;
            let project = LocalKParProject::new_access(abs_path.clone(), KparInnerPath::Root, None);
            let definition = project
                .get_project()?
                .ok_or_else(|| CliError::MissingProject(abs_path.to_string()))?;
            let publisher = definition
                .publisher()
                .ok_or_else(|| CliError::MissingPublisherForUsage(abs_path.to_string()))?;
            let (publisher, name) = parse_project_spelling(
                publisher.to_owned(),
                definition.name().to_owned(),
                abs_path.as_str(),
            )?;
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
            // A resource usage has no `model.toml` form
            if let Some(ProjectDefinition::Toml(_)) = ctx.current_definition() {
                return Err(anyhow!(
                    "`--iri` and `--iri-path` add a resource usage, which a `model.toml` project\n\
                     cannot declare: name the dependency as `<publisher>/<name>`, or use\n\
                     `--dir` or `--kpar-path`"
                )
                .into());
            }
            let iri = iri_or_path_to_iri(iri, iri_path)?;
            pending_override = Some((source_overrides, iri.clone()));
            let usage = InterchangeProjectUsage::Resource {
                resource: iri,
                version_constraint,
            };
            UsageToAdd::Ready(usage.into())
        }
        _ => unreachable!(),
    };

    // Taken before the manifest changes, so that a later failure can put it
    // back.
    let backup = backup_manifest(&ctx)?;

    if no_lock {
        let added = match usage {
            UsageToAdd::Ready(usage) => add_usage(&mut ctx, &usage)?,
            UsageToAdd::AlreadyPresent => false,
            UsageToAdd::PendingIndex(_) => {
                unreachable!("without locking, an index usage is settled from the environment")
            }
        };
        if let Err(error) = record_override(
            pending_override,
            &resolution_opts,
            &mut config,
            config_file,
            no_config,
            &ctx,
            &client,
            &runtime,
            &auth_policy,
            &project_root,
        ) {
            ctx.restore_manifest(&backup)?;
            return Err(error.into());
        }
        return Ok(added);
    }

    let provided_iris = resolution_opts.provided_iris(ProvidedProjects::default());
    if let UsageToAdd::Ready(InterchangeProjectUsageRaw::Resource {
        resource,
        version_constraint: _,
    }) = &usage
        && provided_iris.contains_key(resource.as_str())
    {
        log::info!(
            "{GOOD}note{GOOD:#}: SysMLv2/KerML standard libraries will not be installed during sync"
        );
        // Can't skip either locking or syncing, since lockfile needs to add the usage,
        // and std version being added may affect version resolution of other packages
        // in the dependency graph (e.g. older versions used older std, newer use some
        // newer version)
    }

    // Even when nothing is added, lock and sync, since the environment may
    // be missing or stale
    let (added, pending) = match usage {
        UsageToAdd::Ready(usage) => (add_usage(&mut ctx, &usage)?, None),
        UsageToAdd::AlreadyPresent => (false, None),
        // Added as given, and settled once locked
        UsageToAdd::PendingIndex(pending) => {
            let placeholder = pending.placeholder();
            (
                add_usage(&mut ctx, &placeholder)?,
                Some((placeholder, pending)),
            )
        }
    };
    // Recorded once the usage is in the manifest, and before the
    // resolver is built, which reads the configuration it goes into
    let resolver = record_override(
        pending_override,
        &resolution_opts,
        &mut config,
        config_file,
        no_config,
        &ctx,
        &client,
        &runtime,
        &auth_policy,
        &project_root,
    )
    .and_then(|()| {
        create_resolver(
            resolution_opts,
            &config,
            &ctx,
            provided_iris.clone(),
            client.clone(),
            runtime.clone(),
            auth_policy.clone(),
        )
    })
    .map_err(LockFailure::from);

    let locked = resolver.and_then(|resolver| {
        let respelled = pending
            .as_ref()
            .and_then(|(_, pending)| pending.respelled());
        let lock = lock_scope(resolver, &provided_iris, &ctx, respelled.as_ref())?;
        if let Some((placeholder, pending)) = pending {
            settle_from_lock(&mut ctx, &lock, &placeholder, pending)?;
        }
        Ok::<_, LockFailure<Policy>>(lock)
    });
    let result = locked.and_then(|lock| {
        write_lock_and_sync(
            lock,
            no_sync,
            no_prune,
            client,
            runtime,
            auth_policy,
            &provided_iris,
            &mut ctx,
        )
        .map_err(LockFailure::from)
    });
    match result {
        Ok(()) => Ok(added),
        Err(e) => {
            // Roll back the manifest edit.
            ctx.restore_manifest(&backup)?;
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

/// The manifest a rollback has to put back.
fn backup_manifest(ctx: &ProjectContext) -> Result<ManifestBackup> {
    ctx.backup_manifest().map_err(|error| match error {
        BackupError::NoProject => CliError::MissingProjectCurrentDir.into(),
        error => error.into(),
    })
}

/// Add `usage` to the current project.
fn add_usage(ctx: &mut ProjectContext, usage: &InterchangeProjectUsageRaw) -> Result<bool> {
    Ok(ctx.edit(AddUsage::new(usage)?)?)
}

/// Record the source override `pending` carries, if any, in the
/// configuration file.
///
/// Called once the usage is in the manifest, so a refused `add` leaves no
/// `[[project]]` entry behind.
#[expect(
    clippy::too_many_arguments,
    reason = "takes each independent input as its own parameter"
)]
fn record_override<Policy: HTTPAuthentication>(
    pending: Option<(Box<ProjectSourceOptions>, Iri<String>)>,
    resolution_opts: &ResolutionOptions,
    config: &mut Config,
    config_file: Option<Utf8PathBuf>,
    no_config: bool,
    ctx: &ProjectContext,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
    auth_policy: &Arc<Policy>,
    project_root: &Utf8Path,
) -> Result<()> {
    let Some((source_overrides, iri)) = pending else {
        return Ok(());
    };
    process_overrides(
        resolution_opts,
        source_overrides,
        config,
        config_file,
        no_config,
        ctx,
        client,
        runtime,
        auth_policy,
        project_root,
        &iri,
    )
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
    project_root: &Utf8Path,
    iri: &Iri<String>,
) -> Result<(), anyhow::Error> {
    #[expect(
        clippy::manual_map,
        reason = "the explicit if-let is more readable and compact"
    )]
    let source = if let Some(path) = source_overrides.from_path {
        let metadata = wrapfs::metadata(&path)?;
        // A local directory is used in place, as a `dir` usage is locked.
        if metadata.is_dir() {
            Some(OverrideSource::Editable {
                editable: get_relative(path, project_root)?.as_str().into(),
            })
        } else if metadata.is_file() {
            Some(OverrideSource::LocalKpar {
                kpar_path: get_relative(path, project_root)?.as_str().into(),
            })
        } else {
            bail!("path `{path}` is neither a directory nor a file");
        }
    } else if let Some(url) = source_overrides.from_url {
        let index_urls = resolution_opts.index_urls(config);
        let std_resolver = standard_resolver(
            // TODO: why not use env here?
            None,
            Some(client.clone()),
            index_urls,
            runtime.clone(),
            auth_policy.clone(),
            ctx.cancellation.clone(),
        );
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
            editable: get_relative(editable, project_root)?.as_str().into(),
        })
    } else if let Some(src_path) = source_overrides.as_local_src {
        Some(OverrideSource::LocalSrc {
            src_path: get_relative(src_path, project_root)?.as_str().into(),
        })
    } else if let Some(kpar_path) = source_overrides.as_local_kpar {
        Some(OverrideSource::LocalKpar {
            kpar_path: get_relative(kpar_path, project_root)?.as_str().into(),
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
        let config_path =
            config_file.or_else(|| (!no_config).then(|| ctx.config_root.join(CONFIG_FILE)));

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

/// Replace `placeholder`, the usage `pending` added as given, in the
/// current project: spelled as the project it locked to when it was
/// normalized, and constrained to `^` the locked version when it had no
/// constraint, as `cargo add` does.
fn settle_from_lock(
    ctx: &mut ProjectContext,
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
    let placeholder_identity = normalized_identity(&publisher, &name);
    // It was just locked
    let locked = lock
        .projects
        .iter()
        .find(|p| p.identifiers.iter().any(|id| id == identifier.as_str()))
        .expect("BUG: the usage locked is missing from the lock");
    let (publisher, name) = if recover_spelling {
        // Locking must have checked that the project declares a publisher,
        // and that its publisher and name are an index spelling of
        // `identifier`.
        let locked_publisher = locked
            .publisher
            .clone()
            .expect("BUG: the respelled usage locked to a project with no publisher");
        (
            IndexPublisher::parse(locked_publisher)
                .expect("BUG: the respelled usage locked to a publisher it cannot spell"),
            IndexName::parse(locked.name.clone())
                .expect("BUG: the respelled usage locked to a name it cannot spell"),
        )
    } else {
        (publisher, name)
    };
    let version_constraint = match version_constraint {
        Some(version_constraint) => version_constraint,
        None => VersionReq::parse(&format!("^{}", locked.version))?,
    };
    let settled = InterchangeProjectUsageRaw::Index {
        publisher: publisher.as_str().to_owned(),
        name: name.as_str().to_owned(),
        version_constraint: version_constraint.to_string(),
    };
    let settling = "Settled";
    let header = sysand_core::style::get_style_config().header;
    log::info!("{header}{settling:>12}{header:#} usage: {settled}");
    if settled == *placeholder {
        return Ok(());
    }
    ctx.edit(SettleUsage {
        placeholder: placeholder.clone(),
        placeholder_identity,
        publisher,
        name,
        version_constraint,
    })?;
    Ok(())
}

/// Write `lock` as the lockfile at `ctx`'s state root, and sync unless
/// `no_sync`
#[expect(
    clippy::too_many_arguments,
    reason = "takes each independent input as its own parameter"
)]
fn write_lock_and_sync<Policy: HTTPAuthentication>(
    lock: Lock,
    no_sync: bool,
    no_prune: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    provided_iris: &ProvidedProjects,
    ctx: &mut ProjectContext,
) -> Result<(), anyhow::Error> {
    let lockfile = Lockfile::new(ctx.state_root(), lock);
    lockfile.write()?;
    if !no_sync {
        sync_context(
            lockfile.lock(),
            ctx,
            false,
            false,
            no_prune,
            client,
            provided_iris,
            runtime,
            auth_policy,
            &mut SyncOutcome::default(),
        )?;
    }
    Ok(())
}

/// Lock `ctx`'s scope with `resolver` (see [`create_resolver`]), and sync
/// unless `no_sync`
#[expect(
    clippy::too_many_arguments,
    reason = "takes each independent input as its own parameter"
)]
pub fn resolve_deps<Policy: HTTPAuthentication>(
    no_sync: bool,
    no_prune: bool,
    resolver: CliResolver<Policy>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    provided_iris: ProvidedProjects,
    mut ctx: ProjectContext,
) -> Result<(), anyhow::Error> {
    let lock = lock_scope(resolver, &provided_iris, &ctx, None).map_err(LockFailure::into_error)?;
    write_lock_and_sync(
        lock,
        no_sync,
        no_prune,
        client,
        runtime,
        auth_policy,
        &provided_iris,
        &mut ctx,
    )
}

/// `src_path`, canonicalized and relative to `project_root`.
///
/// `project_root` must be absolute. On Windows, its kind (DOS/UNC)
/// must match the kind of `current_dir()`
fn get_relative<P: AsRef<Utf8Path>>(src_path: P, project_root: &Utf8Path) -> Result<Utf8PathBuf> {
    let path = relativize_path(wrapfs::canonicalize(src_path.as_ref())?, project_root)?;
    if path == "." {
        bail!("cannot add current project as usage of itself");
    }
    Ok(path.into_string().into())
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
