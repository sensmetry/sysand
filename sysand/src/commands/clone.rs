use anyhow::{Result, anyhow, bail};
use fluent_uri::Iri;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use sysand_core::{
    commands::lock::LockOutcome,
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
    cli::{ProjectLocator, ResolutionOptions},
    commands::sync::command_sync,
    get_or_create_env,
};

#[allow(clippy::too_many_arguments)]
pub fn command_clone(
    locator: ProjectLocator,
    version: Option<String>,
    install_path: Option<String>,
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

    let project_path = match install_path {
        Some(p) => {
            wrapfs::create_dir_all(&p)?;
            p
        }
        // TODO: add current_dir to some sort of common params, to
        // avoid calling current_dir everywhere
        // current_dir is desirable over '.', since errors are better,
        // especially if sysand is called from ffi or within some script/app
        // where current dir is not obvious
        // TODO: replace with wrapfs::current_dir once it's merged
        None => std::env::current_dir()?
            .as_path()
            .to_str()
            .unwrap()
            .to_owned(),
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

    let mut local_project = LocalSrcProject {
        project_path: project_path.as_str().into(),
    };

    // TODO: maybe extract to some fn do_* like other fn command_* do?
    let cloning = "Cloning";
    let header = sysand_core::style::get_style_config().header;
    let ProjectLocator {
        auto_location,
        iri,
        from_path,
    } = locator;

    // TODO: maybe clone into temp dir first?
    let (iri_path, info, _meta) = if let Some(auto_location) = auto_location {
        match fluent_uri::Iri::parse(auto_location) {
            Ok(iri) => {
                let (info, meta) = clone_iri(
                    &iri,
                    version,
                    &mut local_project,
                    client.clone(),
                    runtime.clone(),
                    index_urls.clone(),
                    allow_overwrite,
                )?;
                (iri.into_string(), info, meta)
            }
            Err((_e, path)) => {
                let (info, meta) =
                    clone_path(path.as_str().into(), &mut local_project, allow_overwrite)?;
                (path, info, meta)
            }
        }
    } else if let Some(path) = from_path {
        let (info, meta) = clone_path(path.as_str().into(), &mut local_project, allow_overwrite)?;
        (path, info, meta)
    } else if let Some(iri) = iri {
        let (info, meta) = clone_iri(
            &iri,
            version,
            &mut local_project,
            client.clone(),
            runtime.clone(),
            index_urls.clone(),
            allow_overwrite,
        )?;
        (iri.into_string(), info, meta)
    } else {
        unreachable!()
    };
    log::info!(
        "{header}{cloning:>12}{header:#} `{}` (`{}`) {}",
        info.name,
        iri_path, // TODO: maybe canonicalize if path?
        info.version
    );

    if !no_deps {
        let provided_iris = if !include_std {
            // TODO: warn about std libs when resolving deps
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
        let project = EditableProject::new(&project_path, local_project);
        let LockOutcome {
            lock,
            dependencies: _dependencies,
            inputs: _inputs,
        } = sysand_core::commands::lock::do_lock_projects([project], resolver)?;
        // TODO: get_or_create means we tolerate existing env. Is this desirable?
        //       Shouldn't cause any harm
        let mut env = get_or_create_env(&project_path)?;
        command_sync(
            lock,
            project_path.into(),
            &mut env,
            client,
            &provided_iris,
            runtime,
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

    let outcome = resolver.resolve_read(iri)?;
    if let ResolutionOutcome::Resolved(alternatives) = outcome {
        let storage = alternatives
            .into_iter()
            .filter_map(Result::ok)
            .find(|store| {
                version.as_ref().is_none_or(|ver| {
                    store
                        .get_project()
                        .ok()
                        .and_then(|(opt, _)| opt)
                        .is_some_and(|proj| proj.version == *ver)
                })
            })
            .ok_or_else(|| anyhow!(CliError::MissingProject(iri.as_ref().to_string())))?;
        match clone_project(&storage, local_project, allow_overwrite) {
            Ok(m) => Ok(m),
            Err(e) => Err(e.into()),
        }
    } else {
        // TODO: don't eat resolution errors
        bail!(CliError::MissingProject(iri.as_ref().to_string()))
    }
}
