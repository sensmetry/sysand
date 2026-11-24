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
    cancel::CancellationToken,
    context::ProjectContext,
    env::local_directory::LocalDirectoryEnvironment,
    index_location::IndexSource,
    manifest::project::{ProjectFiles, ResolvedProject},
    model::{
        InterchangeProjectChecksumRaw, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        InterchangeProjectUsageRaw,
    },
    project::{
        ProjectDefinition, ProjectRead, any::OverrideProject, local_kpar::KparInnerPath,
        local_src::LocalSrcProject, symbol_index, utils::Identifier,
    },
    resolve::{
        ResolutionInfo,
        file::FileResolverProject,
        memory::{AcceptResource, MemoryResolver},
        priority::PriorityResolver,
        standard::{StandardResolver, standard_resolver},
    },
    style,
    utils::ProvidedIdentifiers,
    workspace::model_toml::Workspace,
};

use anstream::println;
use anyhow::{Result, bail};
use std::{collections::HashSet, sync::Arc};
use sysand_core::{
    info::{InfoProjectError, do_info_project, do_info_usage},
    project::local_kpar::LocalKParProject,
    project::utils::wrapfs,
};

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
    match definition {
        ProjectDefinition::Toml(manifest) => pprint_toml_fields(manifest),
        ProjectDefinition::Json { info: _, meta } => pprint_meta(meta),
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
                | InterchangeProjectUsageRaw::RemoteKpar {
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

/// The `model.toml` keys the shared report leaves out, with `sources`
/// as a count.
fn pprint_toml_fields(manifest: &ResolvedProject) {
    let header = style::get_style_config().header;
    for (key, url) in [
        ("Documentation", &manifest.urls.documentation),
        ("Repository", &manifest.urls.repository),
    ] {
        if let Some(url) = url {
            println!("{header}{key}:{header:#} {url}");
        }
    }
    println!("{header}Metamodel:{header:#} {}", manifest.metamodel);
    if let Some(stdlib) = &manifest.stdlib {
        println!("{header}Stdlib:{header:#} {stdlib}");
    }
    match &manifest.files {
        ProjectFiles::Sources(sources) => println!(
            "{header}Sources:{header:#} {}",
            count(sources.as_slice().len(), "file")
        ),
        ProjectFiles::Root(root) => println!("{header}Root:{header:#} {root}"),
    }
}

/// The `.meta.json` fields, with `index`, `exports` and `checksum` as
/// counts.
fn pprint_meta(meta: &InterchangeProjectMetadataRaw) {
    let header = style::get_style_config().header;
    if let Some(metamodel) = &meta.metamodel {
        println!("{header}Metamodel:{header:#} {metamodel}");
    }
    println!("{header}Created:{header:#} {}", meta.created);
    if let Some(includes_derived) = meta.includes_derived {
        println!("{header}Includes derived:{header:#} {includes_derived}");
    }
    if let Some(includes_implied) = meta.includes_implied {
        println!("{header}Includes implied:{header:#} {includes_implied}");
    }
    if let Some(root) = &meta.root {
        println!("{header}Root:{header:#} {root}");
    }
    if let Some(index) = &meta.index {
        println!("{header}Index:{header:#} {}", count(index.len(), "symbol"));
    }
    if let Some(exports) = &meta.exports {
        println!(
            "{header}Exports:{header:#} {}",
            count(exports.len(), "symbol")
        );
    }
    if let Some(checksum) = &meta.checksum {
        println!(
            "{header}Checksums:{header:#} {}",
            count(checksum.len(), "file")
        );
    }
}

/// `n` followed by `noun`, pluralized with an `s` unless `n` is 1.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
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

/// `info` on the KPAR at `path`: the whole report, or the field `get`.
pub fn command_info_path<P: AsRef<Utf8Path>>(
    path: P,
    get: Option<InfoField>,
    excluded_iris: &ProvidedIdentifiers,
) -> Result<()> {
    let project = interpret_kpar_path(&path)?;
    let definition = do_info_project(&project).map_err(|source| CliError::InvalidProject {
        iri: path.as_ref().to_string(),
        source,
    })?;
    print_info(&project, &definition, get, excluded_iris)
}

/// `info` on the context's current project: the whole report, or the
/// field `get`.
pub fn command_info_current(
    ctx: &ProjectContext,
    project: &LocalSrcProject,
    get: Option<InfoField>,
    excluded_iris: &ProvidedIdentifiers,
) -> Result<()> {
    let definition = ctx
        .current_definition()
        .ok_or_else(|| CliError::InvalidProject {
            iri: project.root_path().to_string(),
            source: InfoProjectError::MissingProject,
        })?;
    print_info(project, definition, get, excluded_iris)
}

/// `info` on the project `resolve` resolves to: the whole report, or the
/// field `get`.
pub fn command_info_resolve<Policy: HTTPAuthentication>(
    resolve: ResolutionInfo,
    get: Option<InfoField>,
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

    let resolver = usage_resolver(
        overrides,
        ctx.env,
        client,
        index_urls,
        runtime,
        auth_policy,
        ctx.cancellation,
    );
    let (definition, project) = do_info_usage(resolve, &resolver)?;
    print_info(&project, &definition, get, excluded_iris)
}

/// The resolver a usage is looked up with: the source overrides (of
/// resource usages only), then `env` and the indexes.
pub fn usage_resolver<Policy: HTTPAuthentication>(
    overrides: Vec<(Identifier, Vec<OverrideProject<Policy>>)>,
    env: Option<LocalDirectoryEnvironment>,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexSource>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    cancellation: CancellationToken,
) -> PriorityResolver<
    MemoryResolver<AcceptResource, OverrideProject<Policy>>,
    StandardResolver<Policy>,
> {
    PriorityResolver::new(
        MemoryResolver::resources_only(overrides),
        standard_resolver(
            env,
            Some(client),
            index_urls,
            runtime,
            auth_policy,
            cancellation,
        ),
    )
}

/// Print `definition`'s report, or the value of its field `get`.
fn print_info<Project: ProjectRead>(
    project: &Project,
    definition: &ProjectDefinition,
    get: Option<InfoField>,
    excluded_iris: &ProvidedIdentifiers,
) -> Result<()> {
    match get {
        None => pprint_project(definition, excluded_iris),
        Some(field) => print_lines(field_lines(field, project, definition)?),
    }
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

/// The lines of one `--get`, one per line, each prefixed with the
/// workspace member they came from, if any.
fn print_lines(lines: Vec<String>) {
    print_member_lines(None, lines);
}

fn print_member_lines(member: Option<&str>, lines: Vec<String>) {
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
            if manifest.root().is_some() {
                log::warn!(
                    "the project has no `index`; a Syside KPAR carries `root` and `exports`"
                );
                Vec::new()
            } else {
                index_lines(&symbol_index(project, manifest)?)
            }
        }
        InfoField::Exports => {
            if manifest.root().is_none() {
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
            .root()
            .into_iter()
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

/// `info` from a workspace root, or from a non-member directory under
/// one: every member in `members` order. Without `--get` each member
/// gets its full block, blank-line separated. With `--get` every line
/// a member emits is prefixed with its name, so a list field stays
/// attributable line by line.
pub fn command_info_workspace(
    workspace: &Workspace,
    field: Option<InfoField>,
    excluded_iris: &HashSet<Identifier>,
) -> Result<()> {
    for (at, member) in workspace.members().iter().enumerate() {
        let definition = ProjectDefinition::Toml(Box::new(member.project.clone()));
        if let Some(field) = field {
            let project =
                LocalSrcProject::new_member(member.dir.clone(), None, member.project.clone());
            let lines = field_lines(field, &project, &definition)?;
            print_member_lines(Some(member.project.name.as_str()), lines);
        } else {
            if at > 0 {
                println!();
            }
            pprint_project(&definition, excluded_iris);
        }
    }

    Ok(())
}
