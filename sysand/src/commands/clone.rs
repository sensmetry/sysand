// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Context as _, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use semver::{Version, VersionReq};

use std::{collections::HashMap, fs, io::ErrorKind, sync::Arc};

use sysand_core::{
    auth::HTTPAuthentication,
    cancel::CancellationToken,
    commands::sync::SyncOutcome,
    config::Config,
    discover::{Ancestry, Seek},
    env::{local_directory::utils::clean_dir, utils::clone_project},
    lock::Lockfile,
    model::InterchangeProjectUsage,
    project::{
        ProjectRead, editable::EditableProject, local_kpar::LocalKParProjectRaw,
        local_src::LocalSrcProject, utils::wrapfs,
    },
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    solve::pubgrub::DEFAULT_INDEX_CONSTRAINT,
    utils::{SP, format_err},
    workspace::model_toml::Selector,
};

use crate::{
    CliError,
    cli::{CloneProjectLocatorArgs, ResolutionOptions},
    commands::sync::sync_context,
    context::{ContextOptions, discover_context},
    style::{GOOD, USAGE},
};

pub enum ProjectLocator {
    Iri(Iri<String>),
    /// An index usage, resolved with its version constraint
    Index {
        resolve: ResolutionInfo,
        version_constraint: VersionReq,
    },
    Dir(Utf8PathBuf),
    KparPath(Utf8PathBuf),
}

/// Clones project from `locator` to `target` directory.
pub fn command_clone<Policy: HTTPAuthentication>(
    locator: CloneProjectLocatorArgs,
    version_constraint: Option<VersionReq>,
    target: Option<Utf8PathBuf>,
    cancellation: CancellationToken,
    no_deps: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let target = target.as_deref().unwrap_or_else(|| Utf8Path::new("."));
    let project_path = {
        // Canonicalization is performed only for better error messages
        let canonical = wrapfs::absolute(target)?;
        match fs::read_dir(target) {
            Ok(mut dir_it) => {
                if dir_it.next().is_some() {
                    bail!("target directory not empty: `{canonical}`")
                }
            }
            Err(e) => match e.kind() {
                ErrorKind::NotFound => {
                    wrapfs::create_dir_all(&canonical)?;
                }
                ErrorKind::NotADirectory => {
                    bail!("target path `{canonical}` is not a directory")
                }
                e => {
                    bail!("failed to get metadata for `{canonical}`: {e}");
                }
            },
        }
        canonical
    };
    if let Some(existing_project) = Ancestry::walk(target, Seek::ProjectToo)?.project() {
        log::warn!(
            "found an existing project in one of target path's parent\n\
            {:>8} directories `{}`",
            ' ',
            existing_project
        );
    }

    let (include_std, locator, local_project, std_resolver) = match obtain_project(
        locator,
        version_constraint,
        resolution_opts,
        config,
        &client,
        &runtime,
        auth_policy.clone(),
        project_path.clone(),
        cancellation.clone(),
    ) {
        Ok(ret) => ret,
        Err(e) => {
            // Safe: the dir was checked to be initially empty, so no
            // unrelated files are affected.
            clean_dir(target);
            return Err(e);
        }
    };

    // The context of the cloned project. A clone creates a project outside
    // any scope, so no workspace is consulted and no lock is taken.
    let mut ctx = discover_context(
        project_path,
        ContextOptions {
            consults_workspace: false,
            selector: Selector::Current,
            needs_a_member: false,
            needs_a_project: true,
            lock_wait: None,
            cancellation,
        },
        |_| None,
    )?
    .ctx;

    if !no_deps {
        let provided_usages = if include_std {
            HashMap::default()
        } else {
            crate::known_std_libs()
        };

        let resolver = PriorityResolver::new(
            MemoryResolver {
                iri_predicate: AcceptAll {},
                projects: provided_usages.clone(),
            },
            std_resolver,
        );
        let project = EditableProject::new(".".into(), local_project);
        let identifiers = match locator {
            ProjectLocator::Iri(iri) => Some(vec![iri]),
            // The `pkg:sysand` PURL the index usage resolved by
            ProjectLocator::Index {
                resolve,
                version_constraint: _,
            } => Some(vec![
                Iri::parse(resolve.id().into_string()).map_err(|(e, _)| anyhow!(e))?,
            ]),
            ProjectLocator::Dir(_) | ProjectLocator::KparPath(_) => None,
        };
        let lock = sysand_core::commands::lock::do_lock_projects(
            [(identifiers, &project)],
            resolver,
            &provided_usages,
            &ctx,
        )?;
        // If we have any std lib dependencies, they will not be installed
        if !provided_usages.is_empty()
            && lock.projects.iter().any(|x| {
                x.identifiers
                    .iter()
                    .any(|y| provided_usages.contains_key(y))
            })
        {
            log::info!(
                "{GOOD}note{GOOD:#}: SysMLv2/KerML standard library packages will not be installed during sync,\n\
                {SP:>5} since they should be provided by the tooling. If you want to install them,\n\
                {SP:>5} run `sysand sync --include-std`"
            );
        }
        let lockfile = Lockfile::new(project.inner().root_path(), lock.canonicalize());
        lockfile.write()?;

        sync_context(
            lockfile.lock(),
            &mut ctx,
            false,
            false,
            false,
            client,
            &provided_usages,
            runtime,
            auth_policy,
            &mut SyncOutcome::default(),
        )?;
    }

    Ok(())
}

fn obtain_project<Policy: HTTPAuthentication>(
    locator: CloneProjectLocatorArgs,
    version_constraint: Option<VersionReq>,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    project_path: Utf8PathBuf,
    cancellation: CancellationToken,
) -> Result<
    (
        bool,
        ProjectLocator,
        LocalSrcProject,
        StandardResolver<Policy>,
    ),
    anyhow::Error,
> {
    let index_urls = resolution_opts.index_urls(config);
    let include_std = resolution_opts.include_std;
    let locator = match locator {
        CloneProjectLocatorArgs {
            identifier: Some((publisher, name)),
            dir: None,
            kpar_path: None,
            iri: None,
        } => {
            // Like the solver, an unconstrained index usage selects every
            // release but no prerelease
            let version_constraint = version_constraint
                .clone()
                .unwrap_or(DEFAULT_INDEX_CONSTRAINT);
            let usage = InterchangeProjectUsage::Index {
                publisher,
                name,
                version_constraint: version_constraint.clone(),
            };
            ProjectLocator::Index {
                resolve: ResolutionInfo::new(usage, None),
                version_constraint,
            }
        }
        CloneProjectLocatorArgs {
            identifier: None,
            dir: Some(dir),
            kpar_path: None,
            iri: None,
        } => ProjectLocator::Dir(dir),
        CloneProjectLocatorArgs {
            identifier: None,
            dir: None,
            kpar_path: Some(kpar_path),
            iri: None,
        } => ProjectLocator::KparPath(kpar_path),
        CloneProjectLocatorArgs {
            identifier: None,
            dir: None,
            kpar_path: None,
            iri: Some(iri),
        } => ProjectLocator::Iri(iri),
        _ => unreachable!(),
    };
    let cloning = "Cloning";
    let cloned = "Cloned";
    let header = sysand_core::style::get_style_config().header;

    // The destination is created, not read, so it is not classified: a
    // clone into a directory under a workspace root is allowed.
    let mut local_project = LocalSrcProject::new_access(project_path, None);

    let std_resolver = standard_resolver(
        None,
        Some(client.clone()),
        index_urls,
        runtime.clone(),
        auth_policy,
        cancellation,
    )?;
    match &locator {
        ProjectLocator::Iri(iri) => {
            log::info!(
                "{header}{cloning:>12}{header:#} project with IRI `{}` to\n\
                {:>12} `{}`",
                iri,
                ' ',
                local_project.root_path(),
            );
            let resolve = ResolutionInfo::iri(iri.to_owned());
            let (_version, storage) =
                get_project_version(&resolve, version_constraint, &std_resolver)?;
            let (info, _meta) = clone_project(&storage, &mut local_project, true)?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
        ProjectLocator::Index {
            resolve,
            version_constraint,
        } => {
            log::info!(
                "{header}{cloning:>12}{header:#} {resolve} to\n\
                {:>12} `{}`",
                ' ',
                local_project.root_path(),
            );
            let (_version, storage) =
                get_project_version(resolve, Some(version_constraint.clone()), &std_resolver)?;
            let (info, _meta) = clone_project(&storage, &mut local_project, true)?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
        ProjectLocator::Dir(path) => {
            if wrapfs::is_file(path)? {
                bail!(
                    "`{path}` is not a directory\n\
                    {USAGE}hint:{USAGE:#} to clone from a KPAR, use `--kpar-path`"
                );
            }
            let remote_project = sysand_core::workspace::model_toml::project_at_path(path, None)?;
            clone_local(
                version_constraint,
                cloning,
                cloned,
                header,
                &mut local_project,
                path,
                remote_project,
            )?;
        }
        ProjectLocator::KparPath(path) => {
            if wrapfs::is_dir(path)? {
                bail!(
                    "`{path}` is not a file\n\
                    {USAGE}hint:{USAGE:#} to clone from a directory, use `--dir`"
                );
            }
            let remote_project = LocalKParProjectRaw::new_guess_root(path)?;
            clone_local(
                version_constraint,
                cloning,
                cloned,
                header,
                &mut local_project,
                path,
                remote_project,
            )?;
        }
    }

    Ok((include_std, locator, local_project, std_resolver))
}

fn clone_local<P: ProjectRead>(
    version_constraint: Option<VersionReq>,
    cloning: &str,
    cloned: &str,
    header: clap::builder::styling::Style,
    local_project: &mut LocalSrcProject,
    path: &Utf8PathBuf,
    remote_project: P,
) -> Result<(), anyhow::Error> {
    if let Some(vc) = version_constraint {
        let version = remote_project
            .get_project()?
            .ok_or_else(|| anyhow!("missing project info"))?
            .version();
        let project_version = Version::parse(&version)
            .with_context(|| format!("project at `{path}` has invalid version `{version}`"))?;
        if !vc.matches(&project_version) {
            bail!(
                "project version {project_version} does not match the given version constraint `{vc}`"
            )
        }
    }
    log::info!(
        "{header}{cloning:>12}{header:#} project from `{}` to\n\
                {:>12} `{}`",
        wrapfs::canonicalize(path)?,
        ' ',
        local_project.root_path(),
    );
    let (info, _meta) = clone_project(&remote_project, local_project, true)?;
    log::info!(
        "{header}{cloned:>12}{header:#} `{}` {}",
        info.name,
        info.version
    );
    Ok(())
}

/// Obtains a project identified by `resolve` via `resolver`. If
/// `version_constraint` is given, obtains the highest version matching it
/// (use `=X.Y.Z` for an exact version). Defaults to `*`
/// (latest non-prerelease)
pub fn get_project_version<R: ResolveRead>(
    resolve: &ResolutionInfo,
    version_constraint: Option<VersionReq>,
    resolver: &R,
) -> Result<(Version, R::ProjectStorage), anyhow::Error> {
    match resolver.resolve_read(resolve)? {
        ResolutionOutcome::Resolved(alternatives) => {
            // Match what the solver does: an unconstrained usage defaults to
            // `*`, but only for candidates whose source poses a version choice
            // in the first place
            let defaults_constraint = match (&version_constraint, resolve.usage()) {
                (Some(_), _) => false,
                (
                    None,
                    InterchangeProjectUsage::Directory {
                        dir: _,
                        publisher: _,
                        name: _,
                    }
                    | InterchangeProjectUsage::KparPath {
                        kpar_path: _,
                        publisher: _,
                        name: _,
                    }
                    | InterchangeProjectUsage::Index {
                        publisher: _,
                        name: _,
                        version_constraint: _,
                    }
                    | InterchangeProjectUsage::Resource {
                        version_constraint: Some(_),
                        resource: _,
                    },
                ) => false,
                (
                    None,
                    InterchangeProjectUsage::Resource {
                        version_constraint: None,
                        resource: _,
                    }
                    | InterchangeProjectUsage::KparUrl {
                        kpar_url: _,
                        publisher: _,
                        name: _,
                    },
                ) => true,
            };
            let mut candidates = Vec::new();
            // Versions only the default ruled out, kept for the error message
            let mut excluded_by_default = Vec::new();
            for alt in alternatives {
                let candidate_project = match alt {
                    Ok(cp) => cp,
                    Err(e) => {
                        // These errors may be ugly, as `candidates` includes all
                        // possible candidates, with expectation that only some
                        // of them will work. So we don't show these by default
                        log::debug!("skipping candidate project: {}", format_err(e));
                        continue;
                    }
                };
                let maybe_definition = match candidate_project.get_project() {
                    Ok(mi) => mi,
                    Err(e) => {
                        log::debug!(
                            "skipping candidate project, failed to get info: {}",
                            format_err(e)
                        );
                        continue;
                    }
                };
                let Some(definition) = maybe_definition else {
                    log::debug!("skipping candidate project with missing info");
                    continue;
                };
                let candidate_version = match Version::parse(&definition.version()) {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!(
                            "skipping candidate project with invalid SemVer version {}: {e}",
                            definition.version()
                        );
                        continue;
                    }
                };
                if let Some(vc) = &version_constraint
                    && !vc.matches(&candidate_version)
                {
                    continue;
                }
                if defaults_constraint
                    && candidate_project.source_may_offer_multiple_versions()
                    && !DEFAULT_INDEX_CONSTRAINT.matches(&candidate_version)
                {
                    log::debug!(
                        "skipping prerelease {candidate_version} of {resolve},\n\
                        due to the default constraint `*`"
                    );
                    excluded_by_default.push(candidate_version);
                    continue;
                }
                candidates.push((candidate_version, candidate_project));
            }

            match candidates.len() {
                0 => match version_constraint {
                    Some(v) => bail!(CliError::MissingProjectVersion(resolve.to_string(), v)),
                    None if !excluded_by_default.is_empty() => {
                        excluded_by_default.sort_unstable_by(|v1, v2| v2.cmp(v1));
                        excluded_by_default.dedup();
                        bail!(CliError::OnlyPrereleaseVersions(
                            resolve.to_string(),
                            excluded_by_default
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
                        ))
                    }
                    None => bail!(CliError::MissingProject(resolve.to_string())),
                },
                1 => Ok(candidates.pop().unwrap()),
                _ => {
                    let max_v = candidates
                        .into_iter()
                        .max_by(|(v1, _), (v2, _)| v1.cmp(v2))
                        .unwrap();
                    Ok(max_v)
                }
            }
        }
        ResolutionOutcome::UnsupportedUsageType { reason } => {
            bail!("locator type of {resolve} is not supported: {reason}")
        }
        ResolutionOutcome::NotFound { reason } => {
            bail!("usage {resolve} was not found: {reason}")
        }
        ResolutionOutcome::Unresolvable { reason } => {
            bail!("usage {resolve} is not resolvable: {reason}")
        }
    }
}

#[cfg(test)]
#[path = "./clone_tests.rs"]
mod tests;
