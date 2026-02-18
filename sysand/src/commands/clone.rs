use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;
use semver::Version;

use std::{collections::HashMap, fs, io::ErrorKind, mem, sync::Arc};

use sysand_core::{
    auth::HTTPAuthentication,
    commands::lock::{DEFAULT_LOCKFILE_NAME, LockOutcome},
    config::Config,
    discover::discover_project,
    env::utils::clone_project,
    project::{ProjectRead, editable::EditableProject, local_src::LocalSrcProject, utils::wrapfs},
    resolve::{
        ResolutionOutcome, ResolveRead,
        memory::{AcceptAll, MemoryResolver},
        priority::PriorityResolver,
        standard::standard_resolver,
    },
};

use crate::{
    CliError, DEFAULT_INDEX_URL,
    cli::{ProjectLocatorArgs, ResolutionOptions},
    commands::sync::command_sync,
    get_or_create_env,
};

pub enum ProjectLocator {
    Iri(Iri<String>),
    Path(String),
}

/// Clones project from `locator` to `target` directory.
#[allow(clippy::too_many_arguments)]
pub fn command_clone<Policy: HTTPAuthentication>(
    locator: ProjectLocatorArgs,
    version: Option<String>,
    target: Option<Utf8PathBuf>,
    no_deps: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std,
    } = resolution_opts;

    let target: Utf8PathBuf = target.unwrap_or_else(|| ".".into());
    let (project_path, cleaner) = {
        // Canonicalization is performed only for better error messages
        let canonical = wrapfs::absolute(&target)?;
        match fs::read_dir(&target) {
            Ok(mut dir_it) => {
                if dir_it.next().is_some() {
                    bail!("target directory not empty: `{}`", canonical)
                }
            }
            Err(e) => match e.kind() {
                ErrorKind::NotFound => {
                    wrapfs::create_dir_all(&canonical)?;
                }
                ErrorKind::NotADirectory => {
                    bail!("target path `{}` is not a directory", canonical)
                }
                e => {
                    bail!("failed to get metadata for `{}`: {}", canonical, e);
                }
            },
        }
        (canonical, DirCleaner(&target))
    };
    if let Some(existing_project) = discover_project(&project_path) {
        log::warn!(
            "found an existing project in one of target path's parent\n\
            {:>8} directories `{}`",
            ' ',
            existing_project.project_path
        );
    }

    let index_urls = if no_index {
        None
    } else {
        Some(config.index_urls(index, vec![DEFAULT_INDEX_URL.to_string()], default_index)?)
    };

    let ProjectLocatorArgs {
        auto_location,
        iri,
        path,
    } = locator;

    let locator = if let Some(auto_location) = auto_location {
        match fluent_uri::Iri::parse(auto_location) {
            Ok(iri) => ProjectLocator::Iri(iri),
            Err((_e, path)) => ProjectLocator::Path(path),
        }
    } else if let Some(path) = path {
        ProjectLocator::Path(path)
    } else if let Some(iri) = iri {
        ProjectLocator::Iri(iri)
    } else {
        unreachable!()
    };

    let cloning = "Cloning";
    let cloned = "Cloned";
    let header = sysand_core::style::get_style_config().header;

    let mut local_project = LocalSrcProject {
        nominal_path: None,
        project_path,
    };
    let std_resolver = standard_resolver(
        None,
        None,
        Some(client.clone()),
        index_urls,
        runtime.clone(),
        auth_policy.clone(),
    );
    match locator {
        ProjectLocator::Iri(iri) => {
            log::info!(
                "{header}{cloning:>12}{header:#} project with IRI `{}` to\n\
                {:>12} `{}`",
                iri,
                ' ',
                local_project.project_path,
            );
            let (_version, storage) = get_project_version(&iri, version, &std_resolver)?;
            let (info, _meta) = clone_project(&storage, &mut local_project, true)?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
        ProjectLocator::Path(path) => {
            let remote_project = LocalSrcProject {
                nominal_path: None,
                project_path: path.into(),
            };
            if let Some(version) = version {
                let project_version = remote_project
                    .get_info()?
                    .ok_or_else(|| anyhow!("missing project info"))?
                    .version;
                if version != project_version {
                    bail!(
                        "given version {version} does not match project version {project_version}"
                    )
                }
            }
            log::info!(
                "{header}{cloning:>12}{header:#} project from `{}` to\n\
                {:>12} `{}`",
                wrapfs::canonicalize(&remote_project.project_path)?,
                ' ',
                local_project.project_path,
            );
            let (info, _meta) = clone_project(&remote_project, &mut local_project, true)?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
    }
    // Project is successfully cloned
    mem::forget(cleaner);

    if !no_deps {
        let provided_iris = if !include_std {
            crate::known_std_libs()
        } else {
            HashMap::default()
        };
        let mut memory_projects = HashMap::default();
        for (k, v) in provided_iris.iter() {
            memory_projects.insert(fluent_uri::Iri::parse(k.clone()).unwrap(), v.to_vec());
        }

        let resolver = PriorityResolver::new(
            MemoryResolver {
                iri_predicate: AcceptAll {},
                projects: memory_projects,
            },
            std_resolver,
        );
        let project = EditableProject::new(".".into(), local_project);
        let LockOutcome {
            lock,
            dependencies: _dependencies,
        } = sysand_core::commands::lock::do_lock_projects([&project], resolver)?;
        // Warn if we have any std lib dependencies
        if !provided_iris.is_empty()
            && lock
                .projects
                .iter()
                .any(|x| x.identifiers.iter().any(|y| provided_iris.contains_key(y)))
        {
            crate::logger::warn_std_deps();
        }
        let lock = lock.canonicalize();
        wrapfs::write(
            project.inner().project_path.join(DEFAULT_LOCKFILE_NAME),
            lock.to_string(),
        )?;

        let mut env = get_or_create_env(&project.inner().project_path)?;
        command_sync(
            &lock,
            &project.inner().project_path,
            &mut env,
            client,
            &provided_iris,
            runtime,
            auth_policy,
        )?;
    }

    Ok(())
}

pub fn get_project_version<R: ResolveRead>(
    iri: &Iri<String>,
    version: Option<String>,
    resolver: &R,
) -> Result<(semver::Version, R::ProjectStorage), anyhow::Error> {
    match resolver.resolve_read(iri)? {
        ResolutionOutcome::Resolved(alternatives) => {
            // If no version is supplied, choose the highest
            // Else, choose version that is supplied
            // TODO: maybe add `no_semver` param to control whether version is
            //       interpreted as semver?
            let requested_version = version
                .as_ref()
                .map(|v| {
                    // TODO: since we require this anyway, might as well take Option<Iri<String>>
                    semver::Version::parse(v)
                        .map_err(|e| anyhow!("failed to parse given version {v} as SemVer: {e}"))
                })
                .transpose()?;
            let mut candidates = Vec::new();
            for alt in alternatives {
                let candidate_project = match alt {
                    Ok(cp) => cp,
                    Err(e) => {
                        // These errors may be ugly, as `candidates` includes all
                        // possible candidates, with expectation that only some
                        // of them will work. So we don't show these by default
                        log::debug!("skipping candidate project: {e}");
                        continue;
                    }
                };
                let maybe_info = match candidate_project.get_info() {
                    Ok(mi) => mi,
                    Err(e) => {
                        log::debug!("skipping candidate project, failed to get info: {e}");
                        continue;
                    }
                };
                let info = match maybe_info {
                    Some(info) => info,
                    None => {
                        log::debug!("skipping candidate project with missing info");
                        continue;
                    }
                };
                let candidate_version = match Version::parse(&info.version) {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!(
                            "skipping candidate project with invalid SemVer version {}: {e}",
                            &info.version
                        );
                        continue;
                    }
                };
                if let Some(version) = &requested_version {
                    if &candidate_version != version {
                        continue;
                    }
                }
                candidates.push((candidate_version, candidate_project));
            }

            match candidates.len() {
                0 => match version {
                    Some(v) => bail!(CliError::MissingProjectVersion(iri.as_ref().to_string(), v)),
                    None => bail!(CliError::MissingProject(iri.as_ref().to_string())),
                },
                1 => {
                    // Can't move out values with match
                    Ok(candidates.pop().unwrap())
                }
                _ => {
                    let max_v = candidates
                        .into_iter()
                        .max_by(|(v1, _), (v2, _)| v1.cmp(v2))
                        .unwrap();
                    Ok(max_v)
                }
            }
        }
        ResolutionOutcome::UnsupportedIRIType(e) => bail!(
            "IRI scheme `{}` of `{}` is not supported: {e}",
            iri.scheme(),
            iri
        ),
        ResolutionOutcome::Unresolvable(e) => {
            bail!("failed to resolve project `{iri}`: {e}")
        }
    }
}

/// Removes all files in the directory on drop. Directory itself
/// is not touched. Use `std::mem::forget()` to prevent drop.
/// This doesn't own `Drop` values, so memory won't be leaked.
struct DirCleaner<'a>(&'a Utf8Path);

impl Drop for DirCleaner<'_> {
    fn drop(&mut self) {
        let Ok(entries) = fs::read_dir(self.0) else {
            return;
        };
        log::debug!("drop: clearing contents of dir `{}`", self.0);

        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(entry_type) = entry.file_type() else {
                continue;
            };
            if entry_type.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
}
