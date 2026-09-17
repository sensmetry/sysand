// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    cell::RefCell,
    io::{IsTerminal as _, Write as _},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};

use sysand_core::{
    auth::HTTPAuthentication,
    commands::migrate::{MigrateOptions, VcsState, check_vcs, migrate_project},
    config::Config,
    context::ProjectContext,
    manifest::usage::{MemberUsage, Usage},
    manifest::{
        MODEL_TOML_FILE, ModelToml, ModelTomlKind, edit::render_new,
        workspace_table::WorkspaceTable,
    },
    model::{
        IndexName, IndexPublisher, InterchangeProjectInfoPartial,
        InterchangeProjectMetadataPartial, META_JSON_FILE, PROJECT_JSON_FILE,
    },
    project::{
        ProjectRead as _,
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::{LocalSrcProject, ProjectFormat, detect_format},
        utils::{relativize_path, validate_manifest, wrapfs},
    },
    resolve::ResolutionInfo,
    utils::ProvidedProjects,
    utils::format_err,
    workspace::model_toml::{discover_table, member_index},
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

    // model-toml.md "JSON to TOML": from a workspace root the command
    // errors unless a path names the project. Decided here rather than
    // through the command scope: putting `migrate` in
    // `consults_the_workspace` would make a discovery error fatal, and an
    // *unlisted* json project under a workspace root is exactly the
    // project a user wants to migrate.
    if path.is_none() && is_workspace_root(&dir)? {
        bail!("run from a workspace root, `migrate` needs the path of the json project to convert");
    }

    // The format decides what happens, but the comparison it implies
    // waits until after the working-tree check: a dirty project with a
    // mismatched pair must hear about the dirty tree first.
    let keep_existing_toml = match detect_format(&dir)? {
        ProjectFormat::Json => false,
        ProjectFormat::Toml => bail!("`{dir}` is already a `model.toml` project"),
        ProjectFormat::Neither => bail!("`{dir}` does not contain a project to migrate"),
        ProjectFormat::Both => true,
    };

    // Before anything is written: `migrate` deletes the json manifests,
    // so a user needs a way back.
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

    if keep_existing_toml {
        // The existing `model.toml` is kept untouched and only the json
        // files are deleted, after checking that both agree on `name`
        // and `version`.
        check_both_agree(&dir)?;
        delete_json_pair(&dir)?;
        let header = sysand_core::style::get_style_config().header;
        let migrated = "Migrated";
        log::info!(
            "{header}{migrated:>12}{header:#} `{dir}`: kept the existing `{MODEL_TOML_FILE}` \
             and removed the json manifests"
        );
        return Ok(());
    }

    // Through the `Partial` types and `validate_manifest`, so unknown
    // keys are warned about rather than dropped silently (model-toml.md
    // "JSON to TOML": "print a warning about unknown fields and
    // discard").
    //
    // The metadata comes first because `validate` takes the metamodel,
    // and the metamodel is in `.meta.json`.
    let meta_text = wrapfs::read_to_string(dir.join(META_JSON_FILE))?;
    let meta: InterchangeProjectMetadataPartial = serde_json::from_str(&meta_text)
        .map_err(|e| anyhow::anyhow!("failed to read `{META_JSON_FILE}`: {e}"))?;
    let meta = validate_manifest(
        dir.join(META_JSON_FILE),
        meta.validate(META_JSON_FILE, None),
    )?;
    let info_text = wrapfs::read_to_string(dir.join(PROJECT_JSON_FILE))?;
    let info: InterchangeProjectInfoPartial = serde_json::from_str(&info_text)
        .map_err(|e| anyhow::anyhow!("failed to read `{PROJECT_JSON_FILE}`: {e}"))?;
    let info = validate_manifest(
        dir.join(PROJECT_JSON_FILE),
        info.validate(PROJECT_JSON_FILE, meta.metamodel.as_deref()),
    )?;

    // Built on the first resource that needs it. `create_resolver` can
    // fail on a malformed index URL or a bad override, and migrating a
    // project with no `resource` usages must not fail on configuration
    // it never reads.
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

    let lookup = |resource: &str| {
        if resource.starts_with("file:") {
            return resolve_file_target(resource, &dir);
        }
        // Before the resolver is built, so that "a standard-library
        // usage needs no network" is structural rather than incidental:
        // the IRI names the release and there is no publisher to ask a
        // target for.
        if sysand_core::model::is_omg_stdlib_iri(resource) {
            return stdlib_target(resource);
        }
        build_resolver()?;
        let borrowed = resolver.borrow();
        let resolver = borrowed.as_ref().expect("just built");
        resolve_remote_target(resource, resolver)
    };
    // model-toml.md "JSON to TOML": "when the command is interactive it
    // may prompt for it instead". Only when stdin is a TTY and
    // `--publisher` is absent -- piped and CI runs get the
    // missing-publisher error.
    let publisher = match publisher {
        Some(publisher) => Some(publisher),
        None if info.publisher.is_none() && std::io::stdin().is_terminal() => {
            prompt_for_publisher()?
        }
        None => None,
    };
    // Checked here as well as in `migrate_project`, so that the message
    // about a value the user typed can name the flag they typed it
    // into. The core check is what a binding caller gets.
    if let Some(value) = &metamodel {
        crate::commands::check_metamodel(value)?;
    }
    let validated = migrate_project(
        &info,
        &meta,
        &MigrateOptions {
            publisher,
            metamodel,
        },
        &lookup,
    )?;
    for warning in &validated.warnings {
        log::warn!("{warning}");
    }

    // model-toml.md "Normalization": a `model.toml` usage is satisfied
    // only by a target that declares the pair as the usage spells it,
    // and a json usage's spellings only ever had to match after
    // normalization. Without this a migrated manifest could be one
    // `sysand lock` refuses -- after the json pair it was converted
    // from has been deleted.
    let mut manifest = validated.value;
    spell_path_usages_as_targets(&mut manifest, &dir);

    // Write first, then delete: a failed write must not leave a project
    // with no manifest at all.
    wrapfs::write(dir.join(MODEL_TOML_FILE), render_new(&manifest).to_string())?;
    delete_json_pair(&dir)?;

    let header = sysand_core::style::get_style_config().header;
    let migrated = "Migrated";
    log::info!("{header}{migrated:>12}{header:#} `{dir}` to `{MODEL_TOML_FILE}`");
    Ok(())
}

/// Whether `dir`'s own `model.toml` defines a workspace.
fn is_workspace_root(dir: &Utf8Path) -> Result<bool> {
    let at = dir.join(MODEL_TOML_FILE);
    if !wrapfs::is_file(&at)? {
        return Ok(false);
    }
    let text = wrapfs::read_to_string(&at)?;
    Ok(matches!(
        ModelToml::peek(MODEL_TOML_FILE, &text),
        Ok(Some(ModelTomlKind::Workspace))
    ))
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

/// The `Both` case: a `model.toml` beside a json pair.
///
/// Neither side goes through `ProjectRead`: `get_project` refuses an
/// ambiguous directory before reading anything.
///
/// Only `name` and `version` are compared. Nothing checks that the two
/// manifests agree about `sources` or usages, so a `model.toml` that agrees on
/// the identity has the json project's usages deleted silently.
fn check_both_agree(dir: &Utf8Path) -> Result<()> {
    let text = wrapfs::read_to_string(dir.join(MODEL_TOML_FILE))?;
    let manifest = ModelToml::parse(MODEL_TOML_FILE, &text).map_err(|report| {
        anyhow::anyhow!("`{dir}/{MODEL_TOML_FILE}` is not a valid manifest:\n{report}")
    })?;
    let ModelToml::Project(project) = manifest.value else {
        bail!(
            "`{dir}/{MODEL_TOML_FILE}` defines a workspace, not a project, so it cannot be the \
             `model.toml` of the json project beside it"
        );
    };

    // An inherited value cannot be resolved without the workspace table,
    // which `migrate` does not load. `publisher` is refused too, though
    // the rule compares only name and version: an unresolvable publisher
    // means the two manifests may name different projects and the
    // comparison could not tell.
    let toml_version = match project.project.version.provided() {
        Some(version) => version.to_string(),
        None => bail!(
            "`{dir}/{MODEL_TOML_FILE}` inherits `version` from a workspace, so it cannot be \
             compared with the json manifest beside it"
        ),
    };
    if project.project.publisher.provided().is_none() {
        bail!(
            "`{dir}/{MODEL_TOML_FILE}` inherits `publisher` from a workspace, so it cannot be \
             compared with the json manifest beside it"
        );
    }

    // Through the `Partial` types and `validate_manifest`, as the Json
    // path above does. Deserializing straight into the strict types
    // would silently accept the unknown keys the `Json` path warns
    // about -- and could not see the usages it is about to delete.
    //
    // The metadata comes first, `validate` taking the metamodel from
    // it. Absent is not an error: `detect_format` calls a directory
    // `Both` when *either* json file is there.
    let meta = match wrapfs::read_to_string(dir.join(META_JSON_FILE)) {
        Ok(text) => {
            let partial: InterchangeProjectMetadataPartial = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("failed to read `{META_JSON_FILE}`: {e}"))?;
            validate_manifest(
                dir.join(META_JSON_FILE),
                partial.validate(META_JSON_FILE, None),
            )?
        }
        Err(_) => sysand_core::model::InterchangeProjectMetadataRaw::default(),
    };
    // A `model.toml` beside a lone `.meta.json` is `Both` -- and it is
    // the state `migrate` itself leaves if it is interrupted between
    // the two deletes, `delete_json_pair` removing `.project.json`
    // first. Nothing readable is lost by finishing the job: a
    // `.meta.json` alone is not a project, and it holds only the fields
    // `warn_discarded` already says a `model.toml` cannot keep. The
    // mirror case -- a lone `.project.json` -- is deleted without
    // comment, and two opposite answers to one shape would be worse than
    // either.
    if !wrapfs::is_file(dir.join(PROJECT_JSON_FILE))? {
        // Nothing to compare against, and nothing to lose: the caller's
        // `delete_json_pair` removes the stray and reports the
        // migration as it does for the mirror case.
        return Ok(());
    }
    let json_text = wrapfs::read_to_string(dir.join(PROJECT_JSON_FILE))?;
    let partial: InterchangeProjectInfoPartial = serde_json::from_str(&json_text)
        .map_err(|e| anyhow::anyhow!("failed to read `{PROJECT_JSON_FILE}`: {e}"))?;
    let json = validate_manifest(
        dir.join(PROJECT_JSON_FILE),
        partial.validate(PROJECT_JSON_FILE, meta.metamodel.as_deref()),
    )?;

    // Identities compare normalized, and `IndexName` derives `PartialEq` on
    // the written string -- so `Lib test` and
    // `lib-test` are the same name and a literal comparison would call
    // them a mismatch.
    let json_name = IndexName::parse(json.name.clone())
        .map_err(|e| anyhow::anyhow!("`{PROJECT_JSON_FILE}` has an invalid `name`: {e}"))?;
    if json_name.normalized() != project.project.name.normalized() {
        bail!(
            "`{MODEL_TOML_FILE}` names `{}` but `{PROJECT_JSON_FILE}` names `{}`; \
             migrating a project that has both requires them to agree",
            project.project.name.as_str(),
            json.name
        );
    }

    // The json side is a string and the toml side a `semver::Version`,
    // so a json version that is not semver is a parse error rather than
    // a disagreement quoting a non-version.
    let json_version = semver::Version::parse(&json.version).map_err(|e| {
        anyhow::anyhow!(
            "`{PROJECT_JSON_FILE}` has version `{}`, which is not a semantic version: {e}",
            json.version
        )
    })?;
    if json_version.to_string() != toml_version {
        bail!(
            "`{MODEL_TOML_FILE}` is version `{toml_version}` but `{PROJECT_JSON_FILE}` is \
             version `{}`; migrating a project that has both requires them to agree",
            json.version
        );
    }

    // Resolving, not only parsing: `metamodel` is required, and the
    // requirement lives in `resolve`. Without this a `model.toml` with
    // no metamodel passes the gate, the json pair is deleted, and what
    // is left is a manifest every command refuses -- with nothing to
    // recover it from. The Json path refuses such a project outright;
    // this is the Both path saying the same thing before it deletes.
    //
    // `None` for the workspace: `migrate` does not load one, which is
    // why the two inherited keys above are refused by hand. Any other
    // marker fails here instead, which is the same answer in one place.
    let resolved = project
        .resolve(None)
        .map_err(|report| {
            anyhow::anyhow!(
                "`{dir}/{MODEL_TOML_FILE}` would not be readable on its own:\n{report}\n\
                 the json manifests beside it are left in place"
            )
        })?
        .into_parts()
        .0;

    warn_about_what_the_json_holds(&resolved, &json, &meta);

    Ok(())
}

/// The filesystem half of a `file:` usage.
///
/// Every step here is a filesystem question, which is why
/// `migrate_project` delegates the whole conversion rather than just
/// the identity: `url::Url::to_file_path` is `cfg`'d off on the wasm
/// target `bindings/js` builds, and `sysand` is never built for it.
fn resolve_file_target(resource: &str, project: &Utf8Path) -> Result<Usage, String> {
    let url = url::Url::parse(resource).map_err(|e| format!("`{resource}` is not a URL: {e}"))?;
    let target = url
        .to_file_path()
        .map_err(|()| format!("`{resource}` is not a path this system can open"))?;
    let target = Utf8PathBuf::from_path_buf(target)
        .map_err(|p| format!("`{}` is not valid UTF-8", p.display()))?;

    // Both project kinds are tried: a project's kind is not decided
    // from its extension.
    let (publisher, name, is_kpar) = read_identity(&target)?;

    // `relativize_path` asserts that both paths are absolute and free
    // of `.`/`..`, so a `sysand migrate ./proj` would abort the process
    // rather than fail. Canonicalize first.
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
    let source = match member_table(target)? {
        // A member resolves its inherited keys against the table, which
        // `get_info` consults; without it, `publisher.workspace = true`
        // is unresolvable and the target reads as broken.
        Some(table) => LocalSrcProject::new_member(target.to_owned(), None, Arc::new(table)),
        None => LocalSrcProject::new_access(target.to_owned(), None),
    };
    // Only "there is no source project here" sends us to the KPAR
    // attempt. A source project that *is* there and cannot be read has
    // its own error, and swallowing it would report a member with an
    // inherited `publisher` as "no project here".
    let info = match source.get_info() {
        Ok(Some(info)) => Some((info, false)),
        Err(error) => return Err(format!("usage target `{target}`: {error}")),
        // Nothing to read as a source project: a project's kind is not
        // decided from its extension, so try a KPAR.
        Ok(None) => {
            let kpar = LocalKParProject::new_access(target.to_owned(), KparInnerPath::Guess, None);
            match kpar.get_info() {
                Ok(Some(info)) => Some((info, true)),
                Ok(None) | Err(_) => None,
            }
        }
    };
    let Some((info, is_kpar)) = info else {
        return Err(format!(
            "`{target}` does not contain a project that `sysand migrate` can read"
        ));
    };

    // The target's own `publisher` is the only source for it, and
    // `model.toml` requires one. The wording is adapted rather than
    // reused from `TargetHasNoPublisher`, whose advice is to run
    // `sysand migrate` on the target -- which is what is running.
    let Some(publisher) = info.publisher else {
        return Err(format!(
            "usage target `{target}` declares no `publisher`, which `model.toml` requires;\n\
             {SP:>8} add one to the target before migrating this project",
            SP = ' '
        ));
    };
    Ok((
        IndexPublisher::parse(publisher).map_err(|e| format!("usage target `{target}`: {e}"))?,
        IndexName::parse(info.name).map_err(|e| format!("usage target `{target}`: {e}"))?,
        is_kpar,
    ))
}

/// The workspace table `target` inherits from, if it is a **member** of
/// one.
///
/// Not `discover_table` alone: that finds any enclosing workspace root
/// without asking whether the target is listed in `members`, and
/// `manifest::inherit` is explicit that "markers resolve only against
/// the workspace the project is a member of. A project that is not a
/// member is outside every workspace regardless of where its directory
/// sits". That population -- an unlisted project under a workspace root
/// -- is exactly the one `migrate` exists for.
///
/// Not `Workspace::load_with` either: it would refuse the whole
/// workspace because some *other* member is broken.
fn member_table(target: &Utf8Path) -> Result<Option<WorkspaceTable>, String> {
    let Some((root, table)) = discover_table(target).map_err(|e| {
        format!(
            "cannot tell whether `{target}` is in a workspace: {}",
            format_err(e)
        )
    })?
    else {
        return Ok(None);
    };
    // `member_index` and not `start_region`: the latter prefix-matches,
    // so a project that merely *sits inside* a member would answer
    // `Member` and be handed the table it is not entitled to. Every
    // "not a member" answer -- unlisted, excluded, nested inside an
    // entry, outside the root -- is an answer here rather than a
    // failure: reporting it would fail the migration of an unrelated
    // project because of a workspace the target merely sits near, and
    // an unlisted project under a root is precisely what `migrate`
    // exists for.
    let index = member_index(&root, &table, target).map_err(|error| {
        format!(
            "cannot place `{target}` in the workspace at `{root}`: {}",
            format_err(error)
        )
    })?;
    Ok(index.map(|_| table))
}

/// Ask for the publisher `model.toml` requires and the json project does
/// not have.
///
/// A plain read, not `rpassword`: that masks what is typed, which is
/// right for a token and wrong for a publisher.
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
/// model-toml.md "Standard library": such a usage is a `kpar-url` that
/// declares no `publisher`, and its name is "the name the KPAR
/// declares" -- which Sysand embeds for every release it knows, and
/// which is not the IRI's stem.
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

    // model-toml.md: "resolved at migration time and pinned to
    // `^<version>`, where `<version>` is the version that resolution
    // found". `get_project_version` is that: it skips candidates that
    // do not read, skips non-semver versions with a warning, and takes
    // the highest. It
    // also hands back a *typed* version, so `^{version}` cannot be
    // built from something that is not one.
    let (version, project) = get_project_version(&resolve, None, resolver)
        .map_err(|e| format!("cannot resolve `{resource}`: {e:#}"))?;

    let info = project
        .get_info()
        .map_err(|e| format!("cannot read the target of `{resource}`: {e}"))?
        .ok_or_else(|| format!("`{resource}` resolves to nothing that declares a project"))?;
    let Some(publisher) = info.publisher else {
        return Err(format!(
            "usage target `{resource}` declares no `publisher`, which `model.toml` requires"
        ));
    };
    let publisher = IndexPublisher::parse(publisher).map_err(|e| format!("`{resource}`: {e}"))?;
    let name = IndexName::parse(info.name).map_err(|e| format!("`{resource}`: {e}"))?;

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

    // model-toml.md: "use as-is for URL". Both a KPAR and a source tree
    // at an `http(s)` URL are written this way -- the specification's
    // rule is unconditional, and the resolver accepts either.
    Ok(Usage::RemoteKpar {
        // A publisher is always written here: a standard-library IRI
        // never reaches this function, `lookup` having answered it
        // offline.
        publisher: Some(publisher),
        name,
        url: fluent_uri::Iri::parse(resource.to_owned())
            .map_err(|(e, _)| format!("`{resource}` is not a valid IRI: {e}"))?,
    })
}

/// Rewrite each typed `dir`/`kpar-path` usage's publisher and name to
/// the spelling its target declares.
///
/// The json spellings only had to match after normalization, so
/// `acme-corp` at a target declaring `Acme Corp` was a valid json
/// project and is not a valid `model.toml` one. The target is on disk
/// and readable here, which is why this lives in the CLI rather than in
/// `migrate_project`: `core` has no filesystem.
fn spell_path_usages_as_targets(manifest: &mut ModelToml, dir: &Utf8Path) {
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
        let target = dir.join(
            relative
                .as_str()
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        // A target that cannot be read leaves the json spelling alone.
        // A json project may name a path that is not checked out, and
        // refusing to migrate it would be a worse answer than a usage
        // `sysand lock` will refuse for the same reason the json one
        // would have: the target is not there.
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

/// Say what the json manifests hold that the `model.toml` does not,
/// before they are deleted.
///
/// The `Both` case is a *success* path whose one failure condition is a
/// name or version mismatch, and `design/model-toml.md` calls migrate
/// "the one command that reads both, and it leaves only the toml one
/// behind". So this warns rather
/// than refusing: what is checked here is not whether the migration is
/// allowed but whether the user knows what it costs.
///
/// Seven things, and not the two obvious ones. The `.meta.json` symbol
/// index and checksums are deliberately **absent**: a `model.toml`
/// cannot hold either (see `warn_discarded`), so comparing them would
/// warn on every `Both` migration including one of migrate's own
/// output. `sources` covers what those two become.
///
/// A json usage is rendered as the json spells it. A `resource` usage
/// has no publisher or name until its target is fetched, and this path
/// builds no resolver -- migrate asks for an index only when the Json
/// path meets a `resource` usage, and adding one here would put a
/// network round trip on a path that otherwise needs none.
fn warn_about_what_the_json_holds(
    toml: &sysand_core::manifest::project::ResolvedProject,
    json: &sysand_core::model::InterchangeProjectInfoRaw,
    meta: &sysand_core::model::InterchangeProjectMetadataRaw,
) {
    let mut lost: Vec<String> = Vec::new();

    if !json.usage.is_empty() && toml.usages.is_empty() {
        for usage in &json.usage {
            lost.push(format!("usage {usage}"));
        }
    }

    // The json side is normalized first: `design/model-toml.md` has
    // migration rewrite `./a.sysml` and `a//b.sysml` into the strict
    // form `sources` takes, so comparing raw strings would report
    // differences the design erases on purpose.
    let mut discard = sysand_core::manifest::diagnostics::Report::new();
    if let Some(from_json) =
        sysand_core::commands::migrate::migrate_sources(meta, Some(&toml.metamodel), &mut discard)
    {
        let held: std::collections::BTreeSet<String> =
            toml.sources.iter().map(ToString::to_string).collect();
        let missing: Vec<String> = from_json
            .iter()
            .map(ToString::to_string)
            .filter(|path| !held.contains(path))
            .collect();
        if !missing.is_empty() {
            lost.push(format!(
                "{} source file{} listed in `{META_JSON_FILE}`: {}",
                missing.len(),
                if missing.len() == 1 { "" } else { "s" },
                missing.join(", ")
            ));
        }
    }

    let scalars: [(&str, Option<String>, bool); 3] = [
        (
            "description",
            json.description.clone(),
            toml.description.is_some(),
        ),
        ("license", json.license.clone(), toml.license.is_some()),
        ("website", json.website.clone(), toml.urls.website.is_some()),
    ];
    for (key, in_json, in_toml) in scalars {
        if let Some(value) = in_json
            && !in_toml
        {
            lost.push(format!("`{key}` (`{value}`)"));
        }
    }

    let lists: [(&str, usize, bool); 2] = [
        (
            "maintainer",
            json.maintainer.len(),
            toml.maintainers.is_empty(),
        ),
        ("topic", json.topic.len(), toml.topics.is_empty()),
    ];
    for (key, count, toml_empty) in lists {
        if count > 0 && toml_empty {
            lost.push(format!(
                "{count} `{key}` entr{}",
                if count == 1 { "y" } else { "ies" }
            ));
        }
    }

    if lost.is_empty() {
        return;
    }

    log::warn!(
        "the json manifests hold what `{MODEL_TOML_FILE}` does not, and are about to be \
         deleted:\n{}\nthe `{MODEL_TOML_FILE}` is kept untouched; copy anything wanted into \
         it first, and re-run `sysand include` for the source list",
        lost.iter()
            .map(|item| format!("             - {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
