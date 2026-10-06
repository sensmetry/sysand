// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
use crate::{
    CliError,
    cli::InfoField,
    style::{GOOD, USAGE},
};
use camino::Utf8Path;
use sysand_core::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    index_location::IndexLocation,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw,
    },
    project::{
        ProjectMut, ProjectRead, any::OverrideProject, local_kpar::KparInnerPath, utils::Identifier,
    },
    resolve::{
        ResolutionInfo, file::FileResolverProject, memory::MemoryResolver,
        priority::PriorityResolver, standard::standard_resolver,
    },
    style,
    utils::{ProvidedIdentifiers, format_err},
};

use anstream::println;
use anyhow::{Result, bail};
use std::{collections::HashSet, sync::Arc};
use sysand_core::{
    info::{do_info_project, do_info_usage},
    project::utils::wrapfs,
    project::{local_kpar::LocalKParProject, local_src::LocalSrcProject},
};

pub fn pprint_interchange_project(
    info: &InterchangeProjectInfoRaw,
    excluded_iris: &ProvidedIdentifiers,
) {
    let header = style::get_style_config().header;
    println!("{header}Name:{header:#} {}", info.name);
    if let Some(publisher) = &info.publisher {
        println!("{header}Publisher:{header:#} {}", publisher);
    }
    if let Some(description) = &info.description {
        println!("{header}Description:{header:#} {}", description);
    }
    println!("{header}Version:{header:#} {}", info.version);
    if let Some(license) = &info.license {
        println!("{header}License:{header:#} {}", license);
    }
    if let Some(website) = &info.website {
        println!("{header}Website:{header:#} {}", website);
    }
    if !info.maintainer.is_empty() {
        println!(
            "{header}Maintainer(s):{header:#} {}",
            info.maintainer.join(", ")
        );
    }
    if !info.topic.is_empty() {
        println!("{header}Topics:{header:#} {}", info.topic.join(", "));
    }
    if info.usage.is_empty() {
        println!("No usages.");
    } else {
        let usages_to_print: Vec<_> = info
            .usage
            .iter()
            .filter(|u| match u {
                InterchangeProjectUsageRaw::Resource { resource, .. } => {
                    !excluded_iris.contains(resource)
                }
                InterchangeProjectUsageRaw::Directory { .. }
                | InterchangeProjectUsageRaw::KparPath { .. }
                | InterchangeProjectUsageRaw::Index { .. } => true,
            })
            .collect();
        let has_ignored_usages = info.usage.len() > usages_to_print.len();
        if has_ignored_usages && usages_to_print.is_empty() {
            // TODO: distinguish between provided iris in general and std libs in
            // particular. Here it's assumed that non-empty excluded_iris == std is ignored,
            // which may not continue to be the case
            println!(
                "All usages are ignored. Standard library usages are not\n\
                shown by default, unless `--include-std` is passed"
            );
        } else {
            println!("{header}Usages:{header:#}");
            for usage in &usages_to_print {
                println!("    {usage}");
            }
            if has_ignored_usages {
                // Same caveat as for "All usages are ignored"
                println!(
                    "Some usages are ignored. Standard library usages are not\n\
                    shown by default, unless `--include-std` is passed"
                );
            }
        }
    }
}

/// How a local project path given on the command line is to be interpreted
#[derive(Clone, Copy, Debug)]
pub enum LocalProjectKind {
    Dir,
    Kpar,
}

fn interpret_project_path<P: AsRef<Utf8Path>>(
    path: P,
    kind: LocalProjectKind,
) -> Result<FileResolverProject> {
    let path = path.as_ref();
    let metadata = wrapfs::metadata(path)?;
    Ok(match kind {
        LocalProjectKind::Dir => {
            if !metadata.is_dir() {
                bail!(
                    "`{path}` is not a directory\n\
                    {USAGE}hint:{USAGE:#} to use a KPAR, use `--kpar-path`"
                );
            }
            FileResolverProject::LocalSrcProject(LocalSrcProject::new_access(path, None))
        }
        LocalProjectKind::Kpar => {
            if !metadata.is_file() {
                bail!(
                    "`{path}` is not a file\n\
                    {USAGE}hint:{USAGE:#} to use a directory, use `--dir`"
                );
            }
            FileResolverProject::LocalKParProject(LocalKParProject::new_access(
                path,
                KparInnerPath::Guess,
                None,
            ))
        }
    })
}

pub fn command_info_path<P: AsRef<Utf8Path>>(
    path: P,
    kind: LocalProjectKind,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    let project = interpret_project_path(&path, kind)?;
    match do_info_project(&project) {
        Ok((info, _)) => {
            pprint_interchange_project(&info, excluded_iris);

            Ok(())
        }
        Err(err) => bail!(CliError::InvalidProject {
            iri: path.as_ref().to_string(),
            source: err
        }),
    }
}

pub fn command_info_resolve<Policy: HTTPAuthentication>(
    resolve: ResolutionInfo,
    _normalise: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexLocation>>,
    excluded_iris: &ProvidedIdentifiers,
    overrides: Vec<(Identifier, Vec<OverrideProject<Policy>>)>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: ProjectContext,
) -> Result<()> {
    // FIXME: The more precise error messages are ignored here. For example,
    // if a user provides a relative file URI (this is invalid since file
    // URIs have to be absolute), the error message will be saying that the
    // interchange project was not found without any hints that the provided
    // URI is invalid.

    let combined_resolver = PriorityResolver::new(
        MemoryResolver::resources_only(overrides),
        standard_resolver(ctx.env, Some(client), index_urls, runtime, auth_policy)?,
    );

    let (info, _) = do_info_usage(resolve, &combined_resolver)?;
    pprint_interchange_project(&info, excluded_iris);
    Ok(())
}

/// Printed whenever the user sets a license, so that they know to add
/// the license texts
pub fn log_license_files_note() {
    log::info!(
        "{GOOD}note{GOOD:#}: every license/exception should have its corresponding\n\
        file in LICENSES/ directory, and for publishing this is required.\n\
        It is recommended to use SPDX license text files from\n\
        https://spdx.org/licenses/ or\n\
        https://github.com/spdx/license-list-data/tree/main/text\n\
        just note that some of them have placeholder copyright\n\
        holder/dates in the text that should be replaced"
    )
}

/// Prints the value of `field` of `project`, a list field one entry per line
pub fn command_info_field<Project: ProjectRead>(project: &Project, field: InfoField) -> Result<()> {
    print_lines(field_lines(
        field,
        || get_info_or_bail(project),
        || get_meta_or_bail(project),
    )?);
    Ok(())
}

pub fn command_info_field_path<P: AsRef<Utf8Path>>(
    path: P,
    kind: LocalProjectKind,
    field: InfoField,
) -> Result<()> {
    command_info_field(&interpret_project_path(&path, kind)?, field)
}

pub fn command_info_field_resolve<Policy: HTTPAuthentication>(
    resolve: ResolutionInfo,
    field: InfoField,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexLocation>>,
    overrides: Vec<(Identifier, Vec<OverrideProject<Policy>>)>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: ProjectContext,
) -> Result<()> {
    let combined_resolver = PriorityResolver::new(
        MemoryResolver::resources_only(overrides),
        standard_resolver(ctx.env, Some(client), index_urls, runtime, auth_policy)?,
    );
    let (info, meta) = do_info_usage(resolve, &combined_resolver)?;
    print_lines(field_lines(field, || Ok(info), || Ok(meta))?);
    Ok(())
}

fn print_lines(lines: Vec<String>) {
    for line in lines {
        println!("{line}");
    }
}

/// The value of `field`, a list field one entry per line, nothing if not set.
/// Only the manifest holding the field is read
fn field_lines(
    field: InfoField,
    info: impl FnOnce() -> Result<InterchangeProjectInfoRaw>,
    meta: impl FnOnce() -> Result<InterchangeProjectMetadataRaw>,
) -> Result<Vec<String>> {
    Ok(match field {
        InfoField::Name => vec![info()?.name],
        InfoField::Publisher => info()?.publisher.into_iter().collect(),
        InfoField::Description => info()?.description.into_iter().collect(),
        InfoField::Version => vec![info()?.version],
        InfoField::License => info()?.license.into_iter().collect(),
        InfoField::Maintainer => info()?.maintainer,
        InfoField::Website => info()?.website.into_iter().collect(),
        InfoField::Topic => info()?.topic,
        InfoField::Usage => info()?.usage.iter().map(ToString::to_string).collect(),
        InfoField::Index => meta()?
            .index
            .into_iter()
            .map(|(symbol, path)| format!("`{symbol}` in `{path}`"))
            .collect(),
        InfoField::Created => vec![meta()?.created],
        InfoField::Metamodel => meta()?.metamodel.into_iter().collect(),
        InfoField::IncludesDerived => meta()?
            .includes_derived
            .map(|x| x.to_string())
            .into_iter()
            .collect(),
        InfoField::IncludesImplied => meta()?
            .includes_implied
            .map(|x| x.to_string())
            .into_iter()
            .collect(),
        InfoField::Checksum => meta()?
            .checksum
            .into_iter()
            .flatten()
            .map(
                |(path, InterchangeProjectChecksumRaw { value, algorithm })| {
                    format!("{algorithm}({path}) = {value}")
                },
            )
            .collect(),
    })
}

pub(crate) fn get_info_or_bail<Project: ProjectRead>(
    project: &Project,
) -> Result<InterchangeProjectInfoRaw> {
    match project.get_info() {
        Ok(Some(info)) => Ok(info),
        Ok(None) => bail!("project does not appear to have a valid `.project.json`"),
        Err(err) => {
            bail!("failed to read `.project.json`: {}", format_err(err))
        }
    }
}

pub(crate) fn get_meta_or_bail<Project: ProjectRead>(
    project: &Project,
) -> Result<InterchangeProjectMetadataRaw> {
    match project.get_meta() {
        Ok(Some(meta)) => Ok(meta),
        Ok(None) => bail!("project does not appear to have a valid `.meta.json`"),
        Err(err) => {
            bail!("failed to read `.meta.json`: {}", format_err(err))
        }
    }
}

pub(crate) fn set_info_or_bail<Project: ProjectMut>(
    project: &mut Project,
    info: &InterchangeProjectInfoRaw,
) -> Result<()> {
    if let Err(err) = project.put_info(info, true) {
        bail!("failed to write `.project.json`: {}", format_err(err));
    }

    Ok(())
}

pub(crate) fn set_meta_or_bail<Project: ProjectMut>(
    project: &mut Project,
    meta: &InterchangeProjectMetadataRaw,
) -> Result<()> {
    if let Err(err) = project.put_meta(meta, true) {
        bail!("failed to write `.meta.json`: {}", format_err(err));
    }

    Ok(())
}
