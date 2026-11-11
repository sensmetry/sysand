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
    index_location::IndexSource,
    manifest::project::ResolvedProject,
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw,
    },
    project::{
        ProjectDefinition, ProjectMut, ProjectRead, any::OverrideProject,
        local_kpar::KparInnerPath, local_src::LocalSrcProject, symbol_index, utils::Identifier,
    },
    resolve::{
        ResolutionInfo,
        file::{FileResolverProject, FileResolverProjectError},
        memory::MemoryResolver,
        priority::PriorityResolver,
        standard::standard_resolver,
    },
    style,
    utils::{ProvidedIdentifiers, format_err},
};

use anstream::println;
use anyhow::{Result, bail};
use std::{collections::HashSet, sync::Arc};
use sysand_core::{
    info::{InfoProjectError, do_info_project, do_info_usage},
    project::local_kpar::LocalKParProject,
    project::utils::wrapfs,
};

/// Report a project: under `--format json` its two json manifests as the
/// `project` and `metadata` documents, or a `model.toml` project's
/// manifest as the `manifest` document; the text summary otherwise.
pub fn report_project(definition: &ProjectDefinition, excluded_iris: &ProvidedIdentifiers) {
    if crate::output::is_json() {
        match definition {
            ProjectDefinition::Json { info, meta } => {
                crate::output::document(
                    "project",
                    serde_json::to_value(info).expect("a manifest serializes"),
                );
                crate::output::document(
                    "metadata",
                    serde_json::to_value(meta).expect("a manifest serializes"),
                );
            }
            ProjectDefinition::Toml(manifest) => {
                crate::output::document("manifest", manifest_json(manifest));
            }
        }
        return;
    }
    pprint_project(definition, excluded_iris);
}

/// A resolved `model.toml` project as JSON, keyed as the manifest is.
fn manifest_json(manifest: &ResolvedProject) -> serde_json::Value {
    serde_json::json!({
        "name": manifest.name.as_str(),
        "publisher": manifest.publisher.as_str(),
        "version": manifest.version.to_string(),
        "description": manifest.description,
        "license": manifest.license.as_ref().map(ToString::to_string),
        "maintainers": manifest.maintainers.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "topics": manifest.topics,
        "metamodel": manifest.metamodel.to_string(),
        "stdlib": manifest.stdlib.as_ref().map(ToString::to_string),
        "root": manifest.root.as_ref().map(|root| root.as_str()),
        "sources": manifest.sources.iter().map(|source| source.as_str()).collect::<Vec<_>>(),
        "urls": {
            "website": manifest.urls.website.as_ref().map(fluent_uri::Iri::as_str),
            "documentation": manifest.urls.documentation.as_ref().map(fluent_uri::Iri::as_str),
            "repository": manifest.urls.repository.as_ref().map(fluent_uri::Iri::as_str),
        },
        "usages": manifest
            .usages
            .iter()
            .map(|located| located.usage.to_string())
            .collect::<Vec<_>>(),
    })
}

fn pprint_project(definition: &ProjectDefinition, excluded_iris: &ProvidedIdentifiers) {
    let header = style::get_style_config().header;
    println!("{header}Name:{header:#} {}", definition.name());
    if let Some(publisher) = definition.publisher() {
        println!("{header}Publisher:{header:#} {}", publisher);
    }
    if let Some(description) = definition.description() {
        println!("{header}Description:{header:#} {}", description);
    }
    println!("{header}Version:{header:#} {}", definition.version());
    if let Some(license) = definition.license() {
        println!("{header}License:{header:#} {}", license);
    }
    if let Some(website) = definition.website() {
        println!("{header}Website:{header:#} {}", website);
    }
    let maintainers = definition.maintainers();
    if !maintainers.is_empty() {
        println!(
            "{header}Maintainer(s):{header:#} {}",
            maintainers.join(", ")
        );
    }
    let topics = definition.topics();
    if !topics.is_empty() {
        println!("{header}Topics:{header:#} {}", topics.join(", "));
    }
    let usages = definition.usages();
    if usages.is_empty() {
        println!("No usages.");
    } else {
        let usages_to_print: Vec<_> = usages
            .iter()
            .filter(|u| match u {
                InterchangeProjectUsageRaw::Resource {
                    resource,
                    version_constraint: _,
                }
                | InterchangeProjectUsageRaw::KparUrl {
                    kpar_url: resource,
                    publisher: _,
                    name: _,
                } => !excluded_iris.contains(resource),
                InterchangeProjectUsageRaw::Directory {
                    dir: _,
                    publisher: _,
                    name: _,
                }
                | InterchangeProjectUsageRaw::KparPath {
                    kpar_path: _,
                    publisher: _,
                    name: _,
                }
                | InterchangeProjectUsageRaw::Index {
                    publisher: _,
                    name: _,
                    version_constraint: _,
                } => true,
            })
            .collect();
        let has_ignored_usages = usages.len() > usages_to_print.len();
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

fn interpret_kpar_path<P: AsRef<Utf8Path>>(path: P) -> Result<FileResolverProject> {
    let path = path.as_ref();
    if !wrapfs::metadata(path)?.is_file() {
        bail!(
            "`{path}` is not a file\n\
            {USAGE}hint:{USAGE:#} to use a directory, use `--dir`"
        );
    }
    Ok(FileResolverProject::LocalKParProject(
        LocalKParProject::new_access(path, KparInnerPath::Guess, None),
    ))
}

pub fn command_info_path<P: AsRef<Utf8Path>>(
    path: P,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    let project = interpret_kpar_path(&path)?;
    report_info(do_info_project(&project), path.as_ref(), excluded_iris)
}

/// Report the context's current project.
pub fn command_info_current(
    ctx: &ProjectContext,
    iri: &Utf8Path,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    let definition = ctx
        .current_definition()
        .cloned()
        .ok_or(InfoProjectError::MissingProject);
    report_info(definition, iri, excluded_iris)
}

fn report_info(
    definition: Result<ProjectDefinition, InfoProjectError<FileResolverProjectError>>,
    iri: &Utf8Path,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    match definition {
        Ok(definition) => {
            report_project(&definition, excluded_iris);

            Ok(())
        }
        Err(err) => bail!(CliError::InvalidProject {
            iri: iri.to_string(),
            source: err
        }),
    }
}

pub fn command_info_resolve<Policy: HTTPAuthentication>(
    resolve: ResolutionInfo,
    _normalise: bool,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexSource>>,
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
        standard_resolver(
            ctx.env,
            Some(client),
            index_urls,
            runtime,
            auth_policy,
            ctx.cancellation,
        )?,
    );

    let (definition, _) = do_info_usage(resolve, &combined_resolver)?;
    report_project(&definition, excluded_iris);
    Ok(())
}

/// Printed whenever the user sets a license.
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
    print_lines(field_lines(field, project, &get_project_or_bail(project)?)?);
    Ok(())
}

/// [`command_info_field`] for the context's current project.
pub fn command_info_current_field(
    ctx: &ProjectContext,
    project: &LocalSrcProject,
    field: InfoField,
) -> Result<()> {
    let Some(definition) = ctx.current_definition() else {
        bail!("project does not appear to have a manifest");
    };
    print_lines(field_lines(field, project, definition)?);
    Ok(())
}

pub fn command_info_field_path<P: AsRef<Utf8Path>>(path: P, field: InfoField) -> Result<()> {
    command_info_field(&interpret_kpar_path(&path)?, field)
}

pub fn command_info_field_resolve<Policy: HTTPAuthentication>(
    resolve: ResolutionInfo,
    field: InfoField,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexSource>>,
    overrides: Vec<(Identifier, Vec<OverrideProject<Policy>>)>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: ProjectContext,
) -> Result<()> {
    let combined_resolver = PriorityResolver::new(
        MemoryResolver::resources_only(overrides),
        standard_resolver(
            ctx.env,
            Some(client),
            index_urls,
            runtime,
            auth_policy,
            ctx.cancellation,
        )?,
    );
    let (definition, project) = do_info_usage(resolve, &combined_resolver)?;
    print_lines(field_lines(field, &project, &definition)?);
    Ok(())
}

/// The lines of one `--get`: one per line in text mode, the `values`
/// document under `--format json`, where `member` names the workspace
/// member they came from, `None` for a single project.
fn print_lines(lines: Vec<String>) {
    print_member_lines(None, lines);
}

fn print_member_lines(member: Option<&str>, lines: Vec<String>) {
    if crate::output::is_json() {
        crate::output::values(member, &lines);
        return;
    }
    for line in lines {
        match member {
            Some(member) => println!("{member}: {line}"),
            None => println!("{line}"),
        }
    }
}

/// The value of `field`, a list field one entry per line, nothing if not set.
fn field_lines<Project: ProjectRead>(
    field: InfoField,
    project: &Project,
    definition: &ProjectDefinition,
) -> Result<Vec<String>> {
    match definition {
        ProjectDefinition::Json { info, meta } => Ok(json_field_lines(field, info, meta)),
        ProjectDefinition::Toml(manifest) => toml_field_lines(field, project, definition, manifest),
    }
}

fn json_field_lines(
    field: InfoField,
    info: &InterchangeProjectInfoRaw,
    meta: &InterchangeProjectMetadataRaw,
) -> Vec<String> {
    match field {
        InfoField::Name => vec![info.name.clone()],
        InfoField::Publisher => info.publisher.iter().cloned().collect(),
        InfoField::Description => info.description.iter().cloned().collect(),
        InfoField::Version => vec![info.version.clone()],
        InfoField::License => info.license.iter().cloned().collect(),
        InfoField::Maintainer => info.maintainer.clone(),
        InfoField::Website => info.website.iter().cloned().collect(),
        InfoField::Topic => info.topic.clone(),
        InfoField::Usage => info.usage.iter().map(ToString::to_string).collect(),
        InfoField::Index => {
            // A root-shaped document has no `index`; the list stays the
            // list a script reads, and the explanation goes to stderr.
            if meta.index.is_none() {
                log::warn!(
                    "the project has no `index`; a Syside KPAR carries `root` and `exports`"
                );
            }
            index_lines(meta.index.iter().flatten())
        }
        InfoField::Created => vec![meta.created.clone()],
        InfoField::Root => meta.root.iter().cloned().collect(),
        InfoField::Exports => {
            // The mirror of `info index` on a root-shaped document: an
            // OMG-shaped one has no `exports`.
            if meta.exports.is_none() {
                log::warn!("the project has no `exports`; an OMG KPAR carries `index`");
            }
            meta.exports.clone().unwrap_or_default()
        }
        InfoField::Metamodel => meta.metamodel.iter().cloned().collect(),
        InfoField::IncludesDerived => meta
            .includes_derived
            .map(|x| x.to_string())
            .into_iter()
            .collect(),
        InfoField::IncludesImplied => meta
            .includes_implied
            .map(|x| x.to_string())
            .into_iter()
            .collect(),
        InfoField::Checksum => meta
            .checksum
            .iter()
            .flatten()
            .map(
                |(path, InterchangeProjectChecksumRaw { value, algorithm })| {
                    format!("{algorithm}({path}) = {value}")
                },
            )
            .collect(),
    }
}

/// `index` and `exports` are computed from the sources, in the shape the
/// project's KPAR would carry. The fields a KPAR records when it is built
/// do not exist in a `model.toml` project.
fn toml_field_lines<Project: ProjectRead>(
    field: InfoField,
    project: &Project,
    definition: &ProjectDefinition,
    manifest: &ResolvedProject,
) -> Result<Vec<String>> {
    Ok(match field {
        InfoField::Name => vec![manifest.name.as_str().to_owned()],
        InfoField::Publisher => vec![manifest.publisher.as_str().to_owned()],
        InfoField::Description => manifest.description.iter().cloned().collect(),
        InfoField::Version => vec![manifest.version.to_string()],
        InfoField::License => manifest.license.iter().map(ToString::to_string).collect(),
        InfoField::Maintainer => manifest
            .maintainers
            .iter()
            .map(ToString::to_string)
            .collect(),
        InfoField::Website => manifest
            .urls
            .website
            .iter()
            .map(|website| website.as_str().to_owned())
            .collect(),
        InfoField::Topic => manifest.topics.clone(),
        InfoField::Usage => definition
            .usages()
            .iter()
            .map(ToString::to_string)
            .collect(),
        InfoField::Index => {
            if manifest.root.is_some() {
                log::warn!(
                    "the project has no `index`; a Syside KPAR carries `root` and `exports`"
                );
                Vec::new()
            } else {
                index_lines(&symbol_index(project, manifest)?)
            }
        }
        InfoField::Exports => {
            if manifest.root.is_none() {
                log::warn!("the project has no `exports`; an OMG KPAR carries `index`");
                Vec::new()
            } else {
                let mut exports: Vec<String> =
                    symbol_index(project, manifest)?.into_keys().collect();
                exports.sort();
                exports
            }
        }
        InfoField::Root => manifest
            .root
            .iter()
            .map(|root| root.as_str().to_owned())
            .collect(),
        InfoField::Metamodel => vec![manifest.metamodel.to_string()],
        InfoField::Created => built_only("created")?,
        InfoField::IncludesDerived => built_only("includes-derived")?,
        InfoField::IncludesImplied => built_only("includes-implied")?,
        InfoField::Checksum => built_only("checksum")?,
    })
}

fn built_only(field: &str) -> Result<Vec<String>> {
    bail!("`{field}` is recorded when the KPAR is built; a `model.toml` project has none")
}

fn index_lines<'a>(index: impl IntoIterator<Item = (&'a String, &'a String)>) -> Vec<String> {
    index
        .into_iter()
        .map(|(symbol, path)| format!("`{symbol}` in `{path}`"))
        .collect()
}

/// A project's manifest, or the reason it has none.
pub(crate) fn get_project_or_bail<Project: ProjectRead>(
    project: &Project,
) -> Result<ProjectDefinition> {
    definition_or_bail(project.get_project())
}

fn definition_or_bail<Error: std::error::Error>(
    read: Result<Option<ProjectDefinition>, Error>,
) -> Result<ProjectDefinition> {
    match read {
        Ok(Some(definition)) => Ok(definition),
        Ok(None) => bail!("project does not appear to have a manifest"),
        Err(err) => bail!("failed to read the project manifest: {}", format_err(err)),
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

/// `info` from a workspace root, or from a non-member directory under
/// one: every member in `members` order. Without `--get` each member
/// gets its full block, blank-line separated. With `--get` every line
/// a member emits is prefixed with its name, so a list field stays
/// attributable line by line.
pub fn command_info_workspace(
    workspace: &sysand_core::workspace::model_toml::Workspace,
    field: Option<InfoField>,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    let table = workspace.table().clone();

    for (at, member) in workspace.members().iter().enumerate() {
        let definition = ProjectDefinition::Toml(Box::new(member.project.clone()));
        if let Some(field) = field {
            let project = LocalSrcProject::new_member(member.dir.clone(), None, table.clone());
            let lines = field_lines(field, &project, &definition)?;
            print_member_lines(Some(member.project.name.as_str()), lines);
        } else {
            if at > 0 {
                crate::output::text(|| println!());
            }
            report_project(&definition, excluded_iris);
        }
    }

    Ok(())
}
