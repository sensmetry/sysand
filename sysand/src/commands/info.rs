// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{
    CliError,
    cli::{
        AddInfoVerb, AddMetaVerb, AddVerb, ClearInfoVerb, ClearMetaVerb, ClearVerb, GetInfoVerb,
        GetMetaVerb, InfoCommandVerb, RemoveInfoVerb, RemoveMetaVerb, RemoveVerb, SetInfoVerb,
        SetMetaVerb, SetVerb,
    },
};
use sysand_core::{
    model::{InterchangeProjectChecksum, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectMut, ProjectRead},
    resolve::{file::FileResolverProject, standard::standard_resolver},
};

use anyhow::{Result, bail};
use fluent_uri::Iri;
use std::{collections::HashSet, env::current_dir, path::Path, sync::Arc};
use sysand_core::{
    info::{do_info, do_info_project},
    project::{local_kpar::LocalKParProject, local_src::LocalSrcProject},
};
use url::Url;

pub fn pprint_interchange_project(
    info: InterchangeProjectInfoRaw,
    excluded_iris: &HashSet<String>,
) {
    println!("Name: {}", info.name);
    if let Some(description) = info.description {
        println!("Description: {}", description);
    }
    println!("Version: {}", info.version);
    if let Some(license) = info.license {
        println!("License: {}", license);
    }
    if let Some(website) = info.website {
        println!("Website: {}", website);
    }
    if !info.maintainer.is_empty() {
        println!("Maintainer(s): {}", info.maintainer.join(", "));
    }
    if !info.topic.is_empty() {
        println!("Topics: {}", info.topic.join(", "));
    }

    if info.usage.is_empty() {
        println!("No usages.");
    } else {
        for usage in info.usage {
            if excluded_iris.contains(&usage.resource) {
                continue;
            }
            print!("    Usage: {}", usage.resource);
            if let Some(v) = usage.version_constraint {
                println!(" ({})", v);
            } else {
                println!();
            }
        }
    }
}

fn interpret_project_path<P: AsRef<Path>>(path: P) -> Result<FileResolverProject> {
    Ok(if path.as_ref().is_file() {
        FileResolverProject::LocalKParProject(LocalKParProject::new_guess_root(path.as_ref())?)
    } else if path.as_ref().is_dir() {
        FileResolverProject::LocalSrcProject(LocalSrcProject {
            project_path: path.as_ref().to_path_buf(),
        })
    } else {
        bail!(CliError::NoResolve(format!(
            "unable to find interchange project at '{}'",
            path.as_ref().display()
        )));
    })
}

pub fn command_info_path<P: AsRef<Path>>(path: P, excluded_iris: &HashSet<String>) -> Result<()> {
    let project = interpret_project_path(&path)?;

    match do_info_project(&project) {
        Some((info, _)) => {
            pprint_interchange_project(info, excluded_iris);

            Ok(())
        }
        None => bail!(CliError::NoResolve(format!(
            "unable to find interchange project at '{}'",
            path.as_ref().display()
        ))),
    }
}

pub fn command_info_uri(
    uri: Iri<String>,
    _normalise: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<Url>>,
    excluded_iris: &HashSet<String>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let cwd = current_dir().ok();

    let local_env_path = Path::new(".").join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

    let combined_resolver = standard_resolver(
        cwd,
        if local_env_path.is_dir() {
            Some(local_env_path)
        } else {
            None
        },
        Some(client),
        index_urls,
        runtime,
    );

    let mut found = false;

    for (info, _) in do_info(&uri, &combined_resolver)? {
        found = true;
        pprint_interchange_project(info, excluded_iris);
    }

    if !found {
        // FIXME: The more precise error messages are ignored here. For example,
        // if a user provides a relative file URI (this is invalid since file
        // URIs have to be absolute), the error message will be saying that the
        // interchange project was not found without any hints that the provided
        // URI is invalid.
        bail!(CliError::NoResolve(format!(
            "unable to find interchange project '{}'",
            uri
        )));
    }

    Ok(())
}

fn print_output(output: Option<Vec<String>>, numbered: bool) {
    if let Some(lines) = output {
        if numbered {
            for (line_number, line) in lines.iter().enumerate() {
                println!("{}: {}", line_number + 1, line);
            }
        } else {
            for line in lines {
                println!("{}", line);
            }
        }
    }
}

pub fn command_info_verb_path<P: AsRef<Path>>(
    path: P,
    verb: InfoCommandVerb,
    numbered: bool,
) -> Result<()> {
    let project = interpret_project_path(&path)?;

    match project {
        FileResolverProject::LocalSrcProject(mut local_src_project) => match verb {
            InfoCommandVerb::Get(get_verb) => apply_get(&get_verb, &local_src_project, numbered),
            InfoCommandVerb::Set(set_verb) => apply_set(&set_verb, &mut local_src_project),
            InfoCommandVerb::Clear(clear_verb) => apply_clear(&clear_verb, &mut local_src_project),
            InfoCommandVerb::Add(add_verb) => apply_add(&add_verb, &mut local_src_project),
            InfoCommandVerb::Remove(remove_verb) => {
                apply_remove(&remove_verb, &mut local_src_project)
            }
        },
        FileResolverProject::LocalKParProject(local_kpar_project) => match verb {
            InfoCommandVerb::Get(get_verb) => apply_get(&get_verb, &local_kpar_project, numbered),
            InfoCommandVerb::Set(_) => bail!("'set' cannot be used with kpar archives"),
            InfoCommandVerb::Clear(_) => bail!("'clear' cannot be used with kpar archives"),
            InfoCommandVerb::Add(_) => bail!("'add' cannot be used with kpar archives"),
            InfoCommandVerb::Remove(_) => bail!("'remove' cannot be used with kpar archives"),
        },
    }
}

pub fn command_info_verb_uri(
    uri: Iri<String>,
    verb: InfoCommandVerb,
    numbered: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<Url>>,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    match verb {
        InfoCommandVerb::Get(get_verb) => {
            let cwd = current_dir().ok();

            let local_env_path =
                Path::new(".").join(sysand_core::env::local_directory::DEFAULT_ENV_NAME);

            let combined_resolver = standard_resolver(
                cwd,
                if local_env_path.is_dir() {
                    Some(local_env_path)
                } else {
                    None
                },
                Some(client),
                index_urls,
                runtime,
            );

            let mut found = false;

            match get_verb {
                crate::cli::GetVerb::GetInfoVerb(get_info_verb) => {
                    for (info, _meta) in do_info(&uri, &combined_resolver)? {
                        found = true;

                        apply_get_info(&get_info_verb, info, numbered)?;
                    }
                }
                crate::cli::GetVerb::GetMetaVerb(get_meta_verb) => {
                    for (_info, meta) in do_info(&uri, &combined_resolver)? {
                        found = true;

                        apply_get_meta(&get_meta_verb, meta, numbered)?;
                    }
                }
            }

            if !found {
                bail!("unable to find a valid project at {}", uri.as_str());
            };
        }
        InfoCommandVerb::Set(_) => bail!("'set' cannot be used with remote projects"),
        InfoCommandVerb::Clear(_) => bail!("'clear' cannot be used with remote projects"),
        InfoCommandVerb::Add(_) => bail!("'add' cannot be used with remote projects"),
        InfoCommandVerb::Remove(_) => bail!("'remove' cannot be used with remote projects"),
    }

    Ok(())
}

pub fn command_info_current_project(
    mut current_project: LocalSrcProject,
    verb: InfoCommandVerb,
    numbered: bool,
) -> Result<()> {
    match verb {
        InfoCommandVerb::Get(get_verb) => apply_get(&get_verb, &current_project, numbered),
        InfoCommandVerb::Set(set_verb) => apply_set(&set_verb, &mut current_project),
        InfoCommandVerb::Clear(clear_verb) => apply_clear(&clear_verb, &mut current_project),
        InfoCommandVerb::Add(add_verb) => apply_add(&add_verb, &mut current_project),
        InfoCommandVerb::Remove(remove_verb) => apply_remove(&remove_verb, &mut current_project),
    }
}

fn get_info_or_bail<Project: ProjectRead>(project: &Project) -> Result<InterchangeProjectInfoRaw> {
    match project.get_info() {
        Ok(Some(info)) => Ok(info),
        Ok(None) => bail!("project does not appear to have a valid .project.json"),
        Err(err) => {
            bail!("failed to read .project.json: {}", err)
        }
    }
}

fn get_meta_or_bail<Project: ProjectRead>(
    project: &Project,
) -> Result<InterchangeProjectMetadataRaw> {
    match project.get_meta() {
        Ok(Some(meta)) => Ok(meta),
        Ok(None) => bail!("project does not appear to have a valid .meta.json"),
        Err(err) => {
            bail!("failed to read .project.json: {}", err)
        }
    }
}

fn set_info_or_bail<Project: ProjectMut>(
    project: &mut Project,
    info: &InterchangeProjectInfoRaw,
) -> Result<()> {
    if let Err(err) = project.put_info(info, true) {
        bail!("failed to write .project.json: {}", err);
    }

    Ok(())
}

fn set_meta_or_bail<Project: ProjectMut>(
    project: &mut Project,
    meta: &InterchangeProjectMetadataRaw,
) -> Result<()> {
    if let Err(err) = project.put_meta(meta, true) {
        bail!("failed to write .meta.json: {}", err);
    }

    Ok(())
}

fn apply_get<Project: ProjectRead>(
    get_verb: &crate::cli::GetVerb,
    project: &Project,
    numbered: bool,
) -> Result<()> {
    match get_verb {
        crate::cli::GetVerb::GetInfoVerb(get_info_verb) => {
            apply_get_info(get_info_verb, get_info_or_bail(project)?, numbered)
        }
        crate::cli::GetVerb::GetMetaVerb(get_meta_verb) => {
            apply_get_meta(get_meta_verb, get_meta_or_bail(project)?, numbered)
        }
    }
}

fn apply_get_info(
    get_info_verb: &GetInfoVerb,
    info: InterchangeProjectInfoRaw,
    numbered: bool,
) -> Result<()> {
    match get_info_verb {
        GetInfoVerb::GetName => print_output(Some(vec![info.name]), numbered),
        GetInfoVerb::GetDescription => print_output(info.description.map(|x| vec![x]), numbered),
        GetInfoVerb::GetVersion => print_output(Some(vec![info.version]), numbered),
        GetInfoVerb::GetLicence => print_output(info.license.map(|x| vec![x]), numbered),
        GetInfoVerb::GetMaintainer => print_output(Some(info.maintainer), numbered),
        GetInfoVerb::GetWebsite => print_output(info.website.map(|x| vec![x]), numbered),
        GetInfoVerb::GetTopic => print_output(Some(info.topic), numbered),
        GetInfoVerb::GetUsage => print_output(
            Some(
                info.usage
                    .into_iter()
                    .map(|usage| {
                        if let Some(version_constraint) = usage.version_constraint {
                            format!("{} ({})", usage.resource, version_constraint)
                        } else {
                            usage.resource.clone()
                        }
                    })
                    .collect(),
            ),
            numbered,
        ),
    }

    Ok(())
}

fn apply_get_meta(
    get_meta_verb: &GetMetaVerb,
    meta: InterchangeProjectMetadataRaw,
    numbered: bool,
) -> Result<()> {
    match get_meta_verb {
        GetMetaVerb::GetIndex => print_output(
            Some(
                meta.index
                    .into_iter()
                    .map(|(symbol, path)| format!("{} in {}", symbol, path))
                    .collect(),
            ),
            numbered,
        ),
        GetMetaVerb::GetCreated => print_output(Some(vec![meta.created]), numbered),
        GetMetaVerb::GetMetamodel => print_output(meta.metamodel.map(|x| vec![x]), numbered),
        GetMetaVerb::GetIncludesDerived => print_output(
            meta.includes_derived.map(|x| vec![format!("{}", x)]),
            numbered,
        ),
        GetMetaVerb::GetIncludesImplied => print_output(
            meta.includes_implied.map(|x| vec![format!("{}", x)]),
            numbered,
        ),
        GetMetaVerb::GetChecksum => print_output(
            meta.checksum.map(|xs| {
                xs.into_iter()
                    .map(|(path, InterchangeProjectChecksum { value, algorithm })| {
                        format!("{}({}) = {}", algorithm, path, value)
                    })
                    .collect()
            }),
            numbered,
        ),
    }

    Ok(())
}

fn apply_set<Project: ProjectRead + ProjectMut>(
    set_verb: &SetVerb,
    project: &mut Project,
) -> Result<()> {
    match set_verb {
        crate::cli::SetVerb::SetInfoVerb(set_info_verb) => {
            let new_info = set_info(set_info_verb, get_info_or_bail(project)?)?;

            set_info_or_bail(project, &new_info)
        }
        crate::cli::SetVerb::SetMetaVerb(set_meta_verb) => {
            let new_meta = set_meta(set_meta_verb, get_meta_or_bail(project)?)?;

            set_meta_or_bail(project, &new_meta)
        }
    }
}

fn set_info(
    set_info_verb: &SetInfoVerb,
    info: InterchangeProjectInfoRaw,
) -> Result<InterchangeProjectInfoRaw> {
    let mut result = info.clone();

    match set_info_verb {
        SetInfoVerb::SetName(value) => {
            result.name = value.clone();
        }
        SetInfoVerb::SetDescription(value) => {
            result.description = Some(value.clone());
        }
        SetInfoVerb::SetVersion(value) => {
            result.version = value.clone();
        }
        SetInfoVerb::SetLicence(value) => {
            result.license = Some(value.clone());
        }
        SetInfoVerb::SetMaintainer(value) => {
            result.maintainer = value.clone();
        }
        SetInfoVerb::SetWebsite(value) => {
            result.website = Some(value.clone());
        }
        SetInfoVerb::SetTopic(value) => {
            result.topic = value.clone();
        }
    }

    Ok(result)
}

fn set_meta(
    set_meta_verb: &SetMetaVerb,
    meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw> {
    let mut result = meta.clone();

    match set_meta_verb {
        SetMetaVerb::SetMetamodel(value) => {
            result.metamodel = Some(value.clone());
        }
        SetMetaVerb::SetIncludesDerived(value) => {
            result.includes_derived = Some(*value);
        }
        SetMetaVerb::SetIncludesImplied(value) => {
            result.includes_implied = Some(*value);
        }
    }

    Ok(result)
}

fn apply_clear<Project: ProjectRead + ProjectMut>(
    clear_verb: &ClearVerb,
    project: &mut Project,
) -> Result<()> {
    match clear_verb {
        crate::cli::ClearVerb::ClearInfoVerb(clear_info_verb) => {
            let new_info = clear_info(clear_info_verb, get_info_or_bail(project)?)?;

            set_info_or_bail(project, &new_info)
        }
        crate::cli::ClearVerb::ClearMetaVerb(clear_meta_verb) => {
            let new_meta = clear_meta(clear_meta_verb, get_meta_or_bail(project)?)?;

            set_meta_or_bail(project, &new_meta)
        }
    }
}

fn clear_info(
    clear_info_verb: &ClearInfoVerb,
    info: InterchangeProjectInfoRaw,
) -> Result<InterchangeProjectInfoRaw> {
    let mut result = info.clone();

    match clear_info_verb {
        ClearInfoVerb::ClearDescription => {
            result.description = None;
        }
        ClearInfoVerb::ClearLicence => {
            result.license = None;
        }
        ClearInfoVerb::ClearMaintainer => {
            result.maintainer = vec![];
        }
        ClearInfoVerb::ClearWebsite => {
            result.website = None;
        }
        ClearInfoVerb::ClearTopic => {
            result.topic = vec![];
        }
    }

    Ok(result)
}

fn clear_meta(
    clear_meta_verb: &ClearMetaVerb,
    meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw> {
    let mut result = meta.clone();

    match clear_meta_verb {
        ClearMetaVerb::ClearMetamodel => {
            result.metamodel = None;
        }
        ClearMetaVerb::ClearIncludesDerived => {
            result.includes_derived = None;
        }
        ClearMetaVerb::ClearIncludesImplied => {
            result.includes_implied = None;
        }
    }

    Ok(result)
}

fn apply_add<Project: ProjectRead + ProjectMut>(
    add_verb: &AddVerb,
    project: &mut Project,
) -> Result<()> {
    match add_verb {
        crate::cli::AddVerb::AddInfoVerb(add_info_verb) => {
            let new_info = add_info(add_info_verb, get_info_or_bail(project)?)?;

            set_info_or_bail(project, &new_info)
        }
        crate::cli::AddVerb::AddMetaVerb(add_meta_verb) => {
            let new_meta = add_meta(add_meta_verb, get_meta_or_bail(project)?)?;

            set_meta_or_bail(project, &new_meta)
        }
    }
}

fn add_info(
    add_info_verb: &AddInfoVerb,
    info: InterchangeProjectInfoRaw,
) -> Result<InterchangeProjectInfoRaw> {
    let mut result = info.clone();

    match add_info_verb {
        AddInfoVerb::AddMaintainer(items) => {
            result.maintainer.extend(items.iter().cloned());
        }
        AddInfoVerb::AddTopic(items) => {
            result.topic.extend(items.iter().cloned());
        }
    }

    Ok(result)
}

fn add_meta(
    add_meta_verb: &AddMetaVerb,
    _meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw> {
    match *add_meta_verb {}
}

fn apply_remove<Project: ProjectRead + ProjectMut>(
    remove_verb: &RemoveVerb,
    project: &mut Project,
) -> Result<()> {
    match remove_verb {
        crate::cli::RemoveVerb::RemoveInfoVerb(remove_info_verb) => {
            let new_info = remove_info(remove_info_verb, get_info_or_bail(project)?)?;

            set_info_or_bail(project, &new_info)
        }
        crate::cli::RemoveVerb::RemoveMetaVerb(remove_meta_verb) => {
            let new_meta = remove_meta(remove_meta_verb, get_meta_or_bail(project)?)?;

            set_meta_or_bail(project, &new_meta)
        }
    }
}

fn remove_info(
    remove_info_verb: &RemoveInfoVerb,
    info: InterchangeProjectInfoRaw,
) -> Result<InterchangeProjectInfoRaw> {
    let mut result = info.clone();

    enum RemoveFailure {
        ZeroIndex,
        EmptyFailure(usize),
        //SingularFailure,
        PluralFailure(usize, usize),
    }

    fn remove(idx: usize, xs: &mut Vec<String>) -> std::result::Result<(), RemoveFailure> {
        if idx == 0 {
            Err(RemoveFailure::ZeroIndex)
        } else if idx > xs.len() {
            if xs.is_empty() {
                Err(RemoveFailure::EmptyFailure(idx))
            }
            /* else if xs.len() == 1 {
                Err(RemoveFailure::SingularFailure)
            } */
            else {
                Err(RemoveFailure::PluralFailure(idx, xs.len()))
            }
        } else {
            xs.remove(idx - 1);
            Ok(())
        }
    }

    match remove_info_verb {
        RemoveInfoVerb::RemoveMaintainer(idx) => {
            if let Err(err) = remove(*idx, &mut result.maintainer) {
                match err {
                    RemoveFailure::ZeroIndex => {
                        bail!("0 is an invalid index, maintainers are indexed from 1")
                    }
                    RemoveFailure::EmptyFailure(idx) => {
                        bail!("trying to remove maintainer {}, but project has none", idx)
                    }
                    RemoveFailure::PluralFailure(idx, len) => bail!(
                        "trying to remove maintainer {}, but project has only {}",
                        idx,
                        len
                    ),
                }
            }
        }
        RemoveInfoVerb::RemoveTopic(idx) => {
            if let Err(err) = remove(*idx, &mut result.topic) {
                match err {
                    RemoveFailure::ZeroIndex => {
                        bail!("0 is an invalid index, topics are indexed from 1")
                    }
                    RemoveFailure::EmptyFailure(idx) => {
                        bail!("trying to remove topic {}, but project has none", idx)
                    }
                    RemoveFailure::PluralFailure(idx, len) => bail!(
                        "trying to remove topic {}, but project has only {}",
                        idx,
                        len
                    ),
                }
            }
        }
    }

    Ok(result)
}

fn remove_meta(
    remove_meta_verb: &RemoveMetaVerb,
    _meta: InterchangeProjectMetadataRaw,
) -> Result<InterchangeProjectMetadataRaw> {
    match *remove_meta_verb {}
}
