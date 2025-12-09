use anyhow::{Result, anyhow, bail};
use fluent_uri::Iri;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use sysand_core::{
    commands::lock::{DEFAULT_LOCKFILE_NAME, LockOutcome},
    config::Config,
    discover::discover_project,
    env::utils::clone_project,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
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
pub fn command_clone(
    locator: ProjectLocatorArgs,
    version: Option<String>,
    target: Option<String>,
    allow_overwrite: bool,
    no_deps: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let ResolutionOptions {
        index,
        default_index,
        no_index,
        include_std,
    } = resolution_opts;

    let project_path = match target {
        Some(p) => {
            let canonical = wrapfs::canonicalize(p)?;
            wrapfs::create_dir_all(&canonical)?;
            canonical
        }
        // TODO: add current_dir to some sort of common params, to
        // avoid calling current_dir everywhere
        // current_dir is desirable over '.', since errors are better,
        // especially if sysand is called from ffi or within some script/app
        // where current dir is not obvious
        // TODO: replace with wrapfs::current_dir once it's merged
        None => std::env::current_dir()?,
    };
    if let Some(existing_project) = discover_project(&project_path) {
        log::warn!(
            "found an existing project in the project destination path or its\n\
            {:>8} parent directory `{}`",
            ' ',
            existing_project.project_path.display()
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

    // TODO: maybe clone into temp dir first?
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
        project_path: project_path.clone(),
    };
    match locator {
        ProjectLocator::Iri(iri) => {
            log::info!("{header}{cloning:>12}{header:#} project with IRI `{}`", iri);
            let (info, _meta) = clone_iri(
                &iri,
                version,
                &mut local_project,
                client.clone(),
                runtime.clone(),
                index_urls.clone(),
                allow_overwrite,
            )?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
        ProjectLocator::Path(path) => {
            log::info!(
                "{header}{cloning:>12}{header:#} project from `{}` to
                {:>12} `{}`",
                wrapfs::canonicalize(&path)?.display(),
                ' ',
                local_project.project_path.display(),
            );
            let (info, _meta) =
                clone_path(path.as_str().into(), &mut local_project, allow_overwrite)?;
            log::info!(
                "{header}{cloned:>12}{header:#} `{}` {}",
                info.name,
                info.version
            );
        }
    }

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
            standard_resolver(
                None,
                None,
                Some(client.clone()),
                index_urls,
                runtime.clone(),
            ),
        );
        let project = EditableProject::new(local_project);
        // TODO: it would be more efficient to use `do_lock_extend()`, as
        //       we have project info/meta
        let LockOutcome {
            lock,
            dependencies: _dependencies,
            inputs: _inputs,
        } = sysand_core::commands::lock::do_lock_projects([project], resolver)?;
        // Warn if we have any std lib dependencies
        if !provided_iris.is_empty()
            && lock
                .projects
                .iter()
                .any(|x| x.identifiers.iter().any(|y| provided_iris.contains_key(y)))
        {
            crate::logger::warn_std_deps();
        }

        let mut env = get_or_create_env(&project_path)?;
        command_sync(
            &lock,
            &project_path,
            &mut env,
            client,
            &provided_iris,
            runtime,
        )?;
        wrapfs::write(
            project_path.join(DEFAULT_LOCKFILE_NAME),
            lock.canonicalize().to_string(),
        )?;
    }

    Ok(())
}

fn clone_path(
    remote_path: PathBuf,
    local_project: &mut LocalSrcProject,
    allow_overwrite: bool,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    let remote_project = LocalSrcProject {
        project_path: remote_path,
    };
    match clone_project(&remote_project, local_project, allow_overwrite) {
        Ok(m) => Ok(m),
        Err(e) => Err(e.into()),
    }
}

fn clone_iri(
    iri: &Iri<String>,
    version: Option<String>,
    local_project: &mut LocalSrcProject,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    index_urls: Option<Vec<url::Url>>,
    allow_overwrite: bool,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    // Here we need to always obtain the project, even if it's one of std libs
    // Different resolver will be used to resolve deps
    let resolver = standard_resolver(None, None, Some(client), index_urls, runtime);

    let (_version, storage) = get_project_version(iri, version, &resolver)?;
    // TODO: we could use move_fs_item, but then the target dir must be new
    //       to not overwrite data
    // match LocalSrcProject::temporary_from_project(&storage) {
    //     Ok((dir, tmp_project)) => ,
    //     Err(_) => todo!(),
    // }
    match clone_project(&storage, local_project, allow_overwrite) {
        Ok(m) => Ok(m),
        Err(e) => Err(e.into()),
    }
}

pub fn get_project_version<R: ResolveRead>(
    iri: &Iri<String>,
    version: Option<String>,
    resolver: &R,
) -> Result<(semver::Version, R::ProjectStorage), anyhow::Error> {
    let outcome = resolver.resolve_read(iri)?;
    Ok(if let ResolutionOutcome::Resolved(alternatives) = outcome {
        // If no version is supplied, choose the highest
        // Else, choose version that is supplied
        // TODO: this eats a whole bunch of potential errors, they should be reported
        //       We also ignore invalid semver versions
        // TODO: maybe add `no_semver` param to control whether version is
        //       interpreted as semver?
        let alt_it = alternatives.into_iter();
        match version {
            Some(version) => {
                let v = semver::Version::parse(&version).map_err(|e| {
                    anyhow!("failed to parse given version {version} as SemVer: {e}")
                })?;
                alt_it
                    .filter_map(|el| {
                        el.ok().and_then(|x| {
                            x.get_info().ok().and_then(|a| {
                                a.and_then(|b| match semver::Version::parse(&b.version) {
                                    Ok(ver) => {
                                        if v == ver {
                                            Some((ver, x))
                                        } else {
                                            None
                                        }
                                    }
                                    Err(_) => None,
                                })
                            })
                        })
                    })
                    .max_by(|el1, el2| el1.0.cmp(&el2.0))
                    .ok_or_else(|| {
                        anyhow!(CliError::MissingProjectVersion(
                            iri.as_ref().to_string(),
                            version
                        ))
                    })?
            }
            None => alt_it
                .filter_map(|el| {
                    el.ok().and_then(|x| {
                        x.get_info().ok().and_then(|a| {
                            a.and_then(|b| match semver::Version::parse(&b.version) {
                                Ok(ver) => Some((ver, x)),
                                Err(_) => None,
                            })
                        })
                    })
                })
                .max_by(|el1, el2| el1.0.cmp(&el2.0))
                .ok_or_else(|| anyhow!(CliError::MissingProject(iri.as_ref().to_string())))?,
        }
    } else {
        // TODO: don't eat resolution errors
        bail!(CliError::MissingProject(iri.as_ref().to_string()))
    })
}
