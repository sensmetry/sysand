// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    cell::RefCell,
    collections::HashSet,
    io::{IsTerminal as _, Write as _},
    sync::Arc,
};
use sysand_core::paths::relativize_path;

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use sysand_core::{
    auth::HTTPAuthentication,
    commands::migrate::{MigrateOptions, VcsState, check_vcs, migrate_project},
    config::Config,
    context::ProjectContext,
    discover::DirManifest,
    manifest::usage::{MemberUsage, Usage},
    manifest::{
        MODEL_TOML_FILE, ModelToml, ModelTomlKind, edit::render_new,
        workspace_table::WorkspaceTable,
    },
    model::{IndexName, IndexPublisher, META_JSON_FILE, PROJECT_JSON_FILE},
    project::{
        ProjectDefinition, ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        utils::wrapfs,
    },
    resolve::ResolutionInfo,
    utils::ProvidedProjects,
    utils::format_err,
    workspace::model_toml::{PathRegion, discover_table, region_of},
};

use crate::{
    cli::ResolutionOptions,
    commands::{
        clone::get_project_version,
        lock::{CliResolver, create_resolver},
    },
};

/// `sysand migrate`: a json project becomes a `model.toml` one.
#[expect(clippy::too_many_arguments, reason = "the resolver's own five")]
pub fn command_migrate<Policy: HTTPAuthentication>(
    path: Option<Utf8PathBuf>,
    publisher: Option<String>,
    metamodel: Option<String>,
    allow_dirty: bool,
    allow_no_vcs: bool,
    resolution_opts: ResolutionOptions,
    config: &Config,
    ctx: &ProjectContext,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    let dir = path
        .clone()
        .unwrap_or_else(|| ctx.current_directory.clone());

    // The context was built at `dir`: the project it holds is the target,
    // and one found above `dir` is not.
    let holds_project = match ctx.current_project() {
        Some(project) => wrapfs::canonicalize(project.root_path())? == wrapfs::canonicalize(&dir)?,
        None => false,
    };
    let definition = match ctx.current_definition() {
        Some(definition) if holds_project => definition,
        _ => {
            // From a workspace root a path must name the project. Unlisted
            // json projects under a workspace root are valid targets, so
            // workspace discovery errors are not fatal for this command.
            if path.is_none()
                && matches!(
                    DirManifest::read(&dir)?,
                    Some(DirManifest::ModelToml { file: _, parsed })
                        if parsed.kind == ModelTomlKind::Workspace
                )
            {
                bail!(
                    "run from a workspace root, `migrate` needs the path of the json project to \
                     convert"
                );
            }
            bail!("`{dir}` does not contain a project to migrate");
        }
    };
    let ProjectDefinition::Json { info, meta } = definition else {
        bail!("`{dir}` is already a `model.toml` project");
    };

    // Before anything is written: the json manifests are deleted, so the
    // user needs a way back.
    if !allow_no_vcs || !allow_dirty {
        match check_vcs(&dir)? {
            VcsState::Clean => {}
            VcsState::Dirty(paths) if !allow_dirty => {
                for changed in &paths {
                    log::debug!("changed: {changed}");
                }
                bail!(
                    "the working tree at `{dir}` has uncommitted changes; commit or stash them \
                     first\nto migrate anyway, pass `--allow-dirty`"
                );
            }
            VcsState::Dirty(_) => {}
            VcsState::NoRepository if !allow_no_vcs => bail!(
                "`{dir}` is not inside a git work tree\nto migrate anyway, pass `--allow-no-vcs`"
            ),
            VcsState::NoRepository => {}
        }
    }

    // Built on the first resource that needs it, so a project with no
    // `resource` usages does not fail on configuration it never reads.
    let resolver: RefCell<Option<CliResolver<Policy>>> = RefCell::new(None);
    let build_resolver = || -> Result<(), String> {
        if resolver.borrow().is_some() {
            return Ok(());
        }
        let built = create_resolver(
            resolution_opts.clone(),
            config,
            ctx,
            ProvidedProjects::default(),
            client.clone(),
            runtime.clone(),
            auth_policy.clone(),
        )
        .map_err(|e| format!("cannot reach an index: {e:#}"))?;
        *resolver.borrow_mut() = Some(built);
        Ok(())
    };

    // The paths of the usages built from a `file:` target, which already
    // carry the spelling that target declares.
    let read_targets: RefCell<HashSet<typed_path::Utf8UnixPathBuf>> = RefCell::default();
    let lookup = |resource: &str| {
        if resource.starts_with("file:") {
            let usage = resolve_file_target(resource, &dir)?;
            if let Some(path) = usage.local_path() {
                read_targets.borrow_mut().insert(path.clone());
            }
            return Ok(usage);
        }
        // Before the resolver is built: the IRI names the release, so a
        // standard-library usage needs no network.
        if sysand_core::model::is_omg_stdlib_iri(resource) {
            return stdlib_target(resource);
        }
        build_resolver()?;
        let borrowed = resolver.borrow();
        let resolver = borrowed.as_ref().expect("just built");
        resolve_remote_target(resource, resolver)
    };
    // Prompt for a missing publisher only when stdin is a TTY and
    // `--publisher` is absent; piped and CI runs get the missing-publisher
    // error.
    let publisher = match publisher {
        Some(publisher) => Some(publisher),
        // Under `--format json` stdout is the document, so there is no
        // prompt: the run is non-interactive.
        None if info.publisher.is_none()
            && std::io::stdin().is_terminal()
            && !crate::output::is_json() =>
        {
            prompt_for_publisher()?
        }
        None => None,
    };
    // Also checked in `migrate_project`; checking here lets the message
    // name the flag.
    if let Some(value) = &metamodel {
        crate::commands::check_metamodel(value)?;
    }
    let validated = migrate_project(
        info,
        meta,
        &MigrateOptions {
            publisher,
            metamodel,
        },
        &lookup,
    )
    .map_err(|report| report.in_file(dir.as_str()))?;
    // A conversion's diagnostics are about the json pair as a whole and
    // carry no position.
    for warning in &validated.warnings {
        log::warn!("{}", warning.message);
    }

    // A `model.toml` usage is satisfied only by a target that declares the
    // pair as the usage spells it, while a json usage only had to match
    // after normalization.
    let mut manifest = validated.value;
    spell_path_usages_as_targets(&mut manifest, &dir, &read_targets.borrow());

    // Write first, then delete: a failed write must not leave a project
    // with no manifest at all.
    wrapfs::write(dir.join(MODEL_TOML_FILE), render_new(&manifest).to_string())?;
    delete_json_pair(&dir)?;

    let header = sysand_core::style::get_style_config().header;
    let migrated = "Migrated";
    log::info!("{header}{migrated:>12}{header:#} `{dir}` to `{MODEL_TOML_FILE}`");
    Ok(())
}

fn delete_json_pair(dir: &Utf8Path) -> Result<()> {
    for json in [PROJECT_JSON_FILE, META_JSON_FILE] {
        let at = dir.join(json);
        if wrapfs::is_file(&at)? {
            wrapfs::remove_file(&at)?;
        }
    }
    Ok(())
}

/// The filesystem half of a `file:` usage.
fn resolve_file_target(resource: &str, project: &Utf8Path) -> Result<Usage, String> {
    let url = url::Url::parse(resource).map_err(|e| format!("`{resource}` is not a URL: {e}"))?;
    let target = url
        .to_file_path()
        .map_err(|()| format!("`{resource}` is not a path this system can open"))?;
    let target = Utf8PathBuf::from_path_buf(target)
        .map_err(|p| format!("`{}` is not valid UTF-8", p.display()))?;

    let (publisher, name, is_kpar) = read_identity(&target)?;

    // `relativize_path` asserts that both paths are absolute and free of
    // `.`/`..` (it would abort on `sysand migrate ./proj`), so
    // canonicalize first.
    let project =
        wrapfs::canonicalize(project).map_err(|e| format!("cannot resolve `{project}`: {e}"))?;
    let target =
        wrapfs::canonicalize(&target).map_err(|e| format!("cannot resolve `{target}`: {e}"))?;
    let relative = relativize_path(&target, &project)
        .map_err(|e| format!("`{target}` cannot be written relative to `{project}`: {e}"))?;

    Ok(if is_kpar {
        Usage::KparPath {
            publisher,
            name,
            kpar_path: relative,
        }
    } else {
        Usage::SourcePath {
            publisher,
            name,
            dir: relative,
        }
    })
}

/// `(publisher, name, is_kpar)` read from the target.
fn read_identity(target: &Utf8Path) -> Result<(IndexPublisher, IndexName, bool), String> {
    // Asking a `LocalSrcProject` about a non-directory fails instead of
    // answering "nothing here" (`<target>/model.toml` on a `.kpar` file is
    // `Not a directory`), so archives are handled before it.
    let is_dir = wrapfs::is_dir(target).map_err(|e| {
        format!(
            "cannot tell whether usage target `{target}` is a directory: {}",
            format_err(e)
        )
    })?;
    if !is_dir {
        let kpar = LocalKParProject::new_access(target.to_owned(), KparInnerPath::Guess, None);
        let definition = match kpar.get_project() {
            Ok(Some(definition)) => Some((definition, true)),
            Ok(None) | Err(_) => None,
        };
        return finish_identity(target, definition);
    }
    let source = match member_table(target)? {
        // A member's inherited keys resolve against the table.
        Some(table) => LocalSrcProject::new_member(target.to_owned(), None, Arc::new(table)),
        None => LocalSrcProject::new_access(target.to_owned(), None),
    };
    // Only "there is no source project here" falls back to a KPAR; a
    // source project that cannot be read is an error.
    let definition = match source.get_project() {
        Ok(Some(definition)) => Some((definition, false)),
        Err(error) => return Err(format!("usage target `{target}`: {error}")),
        // A project's kind is not decided from its extension, so try a KPAR.
        Ok(None) => {
            let kpar = LocalKParProject::new_access(target.to_owned(), KparInnerPath::Guess, None);
            match kpar.get_project() {
                Ok(Some(definition)) => Some((definition, true)),
                Ok(None) | Err(_) => None,
            }
        }
    };
    finish_identity(target, definition)
}

/// The identity half, shared by the directory and archive paths.
fn finish_identity(
    target: &Utf8Path,
    definition: Option<(ProjectDefinition, bool)>,
) -> Result<(IndexPublisher, IndexName, bool), String> {
    let Some((definition, is_kpar)) = definition else {
        return Err(format!(
            "`{target}` does not contain a project that `sysand migrate` can read"
        ));
    };

    // `model.toml` requires a publisher, and the target's own is the only
    // source for it.
    let Some(publisher) = definition.publisher() else {
        return Err(format!(
            "usage target `{target}` declares no `publisher`, which `model.toml` requires;\n\
             {SP:>8} add one to the target before migrating this project",
            SP = ' '
        ));
    };
    Ok((
        IndexPublisher::parse(publisher.to_owned())
            .map_err(|e| format!("usage target `{target}`: {e}"))?,
        IndexName::parse(definition.name().to_owned())
            .map_err(|e| format!("usage target `{target}`: {e}"))?,
        is_kpar,
    ))
}

/// The workspace table `target` inherits from, if it is a **member** of
/// one.
///
/// A project that is not listed in `members` is outside every workspace,
/// wherever its directory sits, so `discover_table` alone is not enough.
/// The workspace is not loaded whole, as another broken member would
/// refuse it.
fn member_table(target: &Utf8Path) -> Result<Option<WorkspaceTable>, String> {
    // A non-directory cannot be a member, and `discover_table` fails on
    // one (`<target>/model.toml` on a `.kpar` file is `Not a directory`).
    if !wrapfs::is_dir(target).map_err(|e| {
        format!(
            "cannot tell whether `{target}` is a directory: {}",
            format_err(e)
        )
    })? {
        return Ok(None);
    }
    let Some((root, table)) = discover_table(target).map_err(|e| {
        format!(
            "cannot tell whether `{target}` is in a workspace: {}",
            format_err(e)
        )
    })?
    else {
        return Ok(None);
    };
    let canonical = wrapfs::canonicalize(target).map_err(|error| {
        format!(
            "cannot place `{target}` in the workspace at `{root}`: {}",
            format_err(error)
        )
    })?;
    // Every "not a member" answer (unlisted, excluded, nested, outside the
    // root) is `None`, not an error.
    Ok(matches!(region_of(&root, &table, &canonical), PathRegion::Member(_)).then_some(table))
}

/// Ask for the publisher `model.toml` requires and the json project does
/// not have.
///
/// The answer is echoed, unlike a token.
fn prompt_for_publisher() -> Result<Option<String>> {
    print!("Publisher for this project (leave empty to abort): ");
    std::io::stdout()
        .flush()
        .context("could not write the prompt")?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("could not read the publisher from the terminal")?;
    let answer = answer.trim();
    Ok((!answer.is_empty()).then(|| answer.to_owned()))
}

/// A usage of an OMG standard-library IRI, without touching the network.
///
/// Such a usage is a `kpar-url` that declares no `publisher`. Its name is
/// the one the KPAR declares, which Sysand embeds for every release it
/// knows, not the IRI's stem.
fn stdlib_target(resource: &str) -> Result<Usage, String> {
    let url = fluent_uri::Iri::parse(resource.to_owned())
        .map_err(|(e, _)| format!("`{resource}` is not a valid IRI: {e}"))?;
    let declared = sysand_core::stdlib::std_lib_name(resource)
        .or_else(|| sysand_core::model::omg_stdlib_stem(resource))
        .ok_or_else(|| format!("`{resource}` names no standard library"))?;
    let name = IndexName::parse(declared).map_err(|e| format!("`{resource}`: {e}"))?;
    Ok(Usage::RemoteKpar {
        publisher: None,
        name,
        url,
    })
}

/// An `http(s)` or unconstrained-`pkg:sysand` usage, through the index.
fn resolve_remote_target<Policy: HTTPAuthentication>(
    resource: &str,
    resolver: &CliResolver<Policy>,
) -> Result<Usage, String> {
    let iri = fluent_uri::Iri::parse(resource.to_owned())
        .map_err(|(e, _)| format!("`{resource}` is not a valid IRI: {e}"))?;
    let resolve = ResolutionInfo::iri(iri);

    // Pinned to `^<version>` of the highest version among the candidates
    // that read, skipping non-semver versions with a warning.
    let (version, project) = get_project_version(&resolve, None, resolver)
        .map_err(|e| format!("cannot resolve `{resource}`: {e:#}"))?;

    let definition = project
        .get_project()
        .map_err(|e| format!("cannot read the target of `{resource}`: {e}"))?
        .ok_or_else(|| format!("`{resource}` resolves to nothing that declares a project"))?;
    let Some(publisher) = definition.publisher().map(ToOwned::to_owned) else {
        return Err(format!(
            "usage target `{resource}` declares no `publisher`, which `model.toml` requires"
        ));
    };
    let publisher = IndexPublisher::parse(publisher).map_err(|e| format!("`{resource}`: {e}"))?;
    let name =
        IndexName::parse(definition.name().to_owned()).map_err(|e| format!("`{resource}`: {e}"))?;

    if sysand_core::purl::parse_sysand_purl(resource)
        .ok()
        .flatten()
        .is_some()
    {
        return Ok(Usage::Index {
            publisher,
            name,
            version_constraint: semver::VersionReq::parse(&format!("^{version}"))
                .map_err(|e| format!("`^{version}` is not a version requirement: {e}"))?,
        });
    }

    // A URL is used as-is; the resolver accepts a KPAR or a source tree.
    Ok(Usage::RemoteKpar {
        // Standard-library IRIs are answered offline before reaching here,
        // so a publisher is always known.
        publisher: Some(publisher),
        name,
        url: fluent_uri::Iri::parse(resource.to_owned())
            .map_err(|(e, _)| format!("`{resource}` is not a valid IRI: {e}"))?,
    })
}

/// Rewrite each typed `dir`/`kpar-path` usage's publisher and name to
/// the spelling its target declares. A usage whose path is in `read` is
/// already spelled that way.
///
/// The json spellings only had to match after normalization, so
/// `acme-corp` at a target declaring `Acme Corp` is a valid json
/// project but not a valid `model.toml` one.
fn spell_path_usages_as_targets(
    manifest: &mut ModelToml,
    dir: &Utf8Path,
    read: &HashSet<typed_path::Utf8UnixPathBuf>,
) {
    let ModelToml::Project(project) = manifest else {
        return;
    };
    for entry in &mut project.inline_usages {
        let MemberUsage::Declared(usage) = entry else {
            continue;
        };
        let relative = match usage {
            Usage::SourcePath {
                dir,
                publisher: _,
                name: _,
            } => dir.clone(),
            Usage::KparPath {
                kpar_path,
                publisher: _,
                name: _,
            } => kpar_path.clone(),
            _ => continue,
        };
        if read.contains(&relative) {
            continue;
        }
        let target = dir.join(
            relative
                .as_str()
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        // A target that cannot be read leaves the json spelling alone: a
        // json project may name a path that is not checked out.
        let (publisher, name, _) = match read_identity(&target) {
            Ok(identity) => identity,
            Err(error) => {
                log::warn!(
                    "`{relative}` cannot be read, so its usage keeps the spelling the json \
                     project gave it: {error}"
                );
                continue;
            }
        };
        match usage {
            Usage::SourcePath {
                publisher: p,
                name: n,
                dir: _,
            }
            | Usage::KparPath {
                publisher: p,
                name: n,
                kpar_path: _,
            } if *p != publisher || *n != name => {
                log::info!(
                    "usage `{p}/{n}` is written as `{publisher}/{name}`, which is how its \
                     target declares it"
                );
                *p = publisher;
                *n = name;
            }
            _ => {}
        }
    }
}
