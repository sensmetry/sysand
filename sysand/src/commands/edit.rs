// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use sysand_core::{
    context::{EditError, ManifestStep, ProjectContext},
    manifest::{
        common::Maintainer,
        edit::{self, Array, DocumentMut, ListKey, ProjectKey, Value},
    },
    model::{
        IndexName, IndexPublisher, InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw,
        KnownMetamodel, MetamodelFamily,
    },
    project::ProjectDefinition,
    utils::format_err,
};

use crate::{
    cli::{EditArgs, MetamodelKindCli},
    commands::info::{log_license_files_note, set_info_or_bail, set_meta_or_bail},
};

/// A failure of an edit, carried through the context's write path.
#[derive(Debug, thiserror::Error)]
#[error("{0:#}")]
struct EditFailed(anyhow::Error);

/// Applies `edits` to the context's current project. Each of `.project.json`
/// and `.meta.json` is written (once) only if one of its fields is edited.
pub fn command_edit(ctx: &mut ProjectContext, edits: EditArgs) -> Result<()> {
    let (edits_info, edits_meta) = (edits_info(&edits), edits_meta(&edits));
    if !edits_info && !edits_meta {
        unreachable!()
    }
    let definition = ctx
        .current_definition()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("project does not appear to have a manifest"))?;
    if matches!(definition, ProjectDefinition::Toml(_)) {
        refuse_computed_fields(&edits)?;
        ctx.edit_manifest(|document| edit_toml(document, &edits).map_err(EditFailed))
            .map_err(toml_edit_error)?;
        if edits.license.is_some() {
            log_license_files_note();
        }
        return Ok(());
    }
    let ProjectDefinition::Json { mut info, mut meta } = definition else {
        unreachable!("a definition is a `model.toml` or a json pair")
    };
    if edits_info {
        edit_info(&edits, &mut info)?;
    }
    if edits_meta {
        edit_meta(&edits, &mut meta);
    }
    ctx.edit_project(|project| -> Result<(), EditFailed> {
        if edits_info {
            set_info_or_bail(project, &info).map_err(EditFailed)?;
        }
        if edits_meta {
            set_meta_or_bail(project, &meta).map_err(EditFailed)?;
        }
        Ok(())
    })
    .map_err(|error| match error {
        EditError::Edit(EditFailed(error)) => error,
        error => error.into(),
    })?;
    if edits.license.is_some() {
        log_license_files_note();
    }
    Ok(())
}

/// The failure of a `model.toml` edit, spelled as the manifest read and
/// write errors always were.
fn toml_edit_error(error: EditError<ManifestStep<EditFailed>>) -> anyhow::Error {
    match error {
        EditError::Edit(ManifestStep::Read(error)) => {
            anyhow::anyhow!("failed to read `model.toml`: {}", format_err(error))
        }
        EditError::Edit(ManifestStep::Write(error)) => {
            anyhow::anyhow!("failed to write `model.toml`: {}", format_err(error))
        }
        EditError::Edit(ManifestStep::Edit(EditFailed(error))) => error,
        EditError::Edit(ManifestStep::Invalid(error)) => anyhow::Error::new(error),
        error => error.into(),
    }
}

fn edits_info(edits: &EditArgs) -> bool {
    let EditArgs {
        name,
        publisher,
        version,
        description,
        clear_description,
        license,
        clear_license,
        website,
        clear_website,
        add_maintainer,
        remove_maintainer,
        clear_maintainers,
        add_topic,
        remove_topic,
        clear_topics,
        metamodel: _,
        metamodel_release: _,
        metamodel_release_custom: _,
        custom_metamodel: _,
        clear_metamodel: _,
        root: _,
        clear_root: _,
        includes_derived: _,
        clear_includes_derived: _,
        includes_implied: _,
        clear_includes_implied: _,
    } = edits;
    name.is_some()
        || publisher.is_some()
        || version.is_some()
        || description.is_some()
        || *clear_description
        || license.is_some()
        || *clear_license
        || website.is_some()
        || *clear_website
        || !add_maintainer.is_empty()
        || !remove_maintainer.is_empty()
        || *clear_maintainers
        || !add_topic.is_empty()
        || !remove_topic.is_empty()
        || *clear_topics
}

fn edits_meta(edits: &EditArgs) -> bool {
    let EditArgs {
        metamodel,
        custom_metamodel,
        clear_metamodel,
        root,
        clear_root,
        includes_derived,
        clear_includes_derived,
        includes_implied,
        clear_includes_implied,
        name: _,
        publisher: _,
        version: _,
        description: _,
        clear_description: _,
        license: _,
        clear_license: _,
        website: _,
        clear_website: _,
        add_maintainer: _,
        remove_maintainer: _,
        clear_maintainers: _,
        add_topic: _,
        remove_topic: _,
        clear_topics: _,
        metamodel_release: _,
        metamodel_release_custom: _,
    } = edits;
    metamodel.is_some()
        || custom_metamodel.is_some()
        || *clear_metamodel
        || root.is_some()
        || *clear_root
        || includes_derived.is_some()
        || *clear_includes_derived
        || includes_implied.is_some()
        || *clear_includes_implied
}

fn edit_info(edits: &EditArgs, info: &mut InterchangeProjectInfoRaw) -> Result<()> {
    if let Some(name) = &edits.name {
        info.name = name.as_str().to_owned();
    }
    if let Some(publisher) = &edits.publisher {
        info.publisher = Some(publisher.as_str().to_owned());
    }
    if let Some(version) = &edits.version {
        info.version = version.to_string();
    }
    if let Some(description) = &edits.description {
        info.description = Some(description.clone());
    } else if edits.clear_description {
        info.description = None;
    }
    if let Some(license) = &edits.license {
        info.license = Some(license.to_string());
    } else if edits.clear_license {
        info.license = None;
    }
    if let Some(website) = &edits.website {
        info.website = Some(website.to_string());
    } else if edits.clear_website {
        info.website = None;
    }
    edit_list(
        "maintainer",
        &mut info.maintainer,
        edits.clear_maintainers,
        &edits.remove_maintainer,
        &edits.add_maintainer,
    )?;
    edit_list(
        "topic",
        &mut info.topic,
        edits.clear_topics,
        &edits.remove_topic,
        &edits.add_topic,
    )?;
    Ok(())
}

/// Clears `list` (if `clear`), then removes every entry equal to one of
/// `remove`, then appends `add`. A value to remove that is not in the list
/// is an error.
fn edit_list(
    kind: &str,
    list: &mut Vec<String>,
    clear: bool,
    remove: &[String],
    add: &[String],
) -> Result<()> {
    if clear {
        list.clear();
    }
    for value in remove {
        let len = list.len();
        list.retain(|entry| entry != value);
        if list.len() == len {
            bail!("project has no {kind} `{value}`");
        }
    }
    list.extend(add.iter().cloned());
    Ok(())
}

/// Refuses the fields a build computes (`index`, `checksum`, `created`,
/// `includesDerived`, `includesImplied`) on a toml project.
///
/// `sysand edit` has no flag for the first three, so only the `includes*`
/// fields can reach here. The manifest has no room for them, so the refusal
/// is permanent.
fn refuse_computed_fields(edits: &EditArgs) -> Result<()> {
    if edits.includes_derived.is_some() {
        refuse_computed_field("includes-derived", "set")?;
    }
    if edits.clear_includes_derived {
        refuse_computed_field("includes-derived", "cleared")?;
    }
    if edits.includes_implied.is_some() {
        refuse_computed_field("includes-implied", "set")?;
    }
    if edits.clear_includes_implied {
        refuse_computed_field("includes-implied", "cleared")?;
    }
    Ok(())
}

fn refuse_computed_field(field: &str, verb: &str) -> Result<()> {
    bail!(
        "`{field}` is computed when the KPAR is built\n\
        {SP:>8} and cannot be {verb} on a `model.toml` project",
        SP = ' '
    );
}

fn edit_meta(edits: &EditArgs, meta: &mut InterchangeProjectMetadataRaw) {
    if let Some(kind) = edits.metamodel {
        meta.metamodel = Some(omg_metamodel(kind, edits).to_string());
    } else if let Some(metamodel) = &edits.custom_metamodel {
        meta.metamodel = Some(metamodel.clone());
    } else if edits.clear_metamodel {
        meta.metamodel = None;
    }
    if let Some(root) = &edits.root {
        meta.root = Some(root.to_string());
    } else if edits.clear_root {
        meta.root = None;
    }
    if let Some(includes_derived) = edits.includes_derived {
        meta.includes_derived = Some(includes_derived);
    } else if edits.clear_includes_derived {
        meta.includes_derived = None;
    }
    if let Some(includes_implied) = edits.includes_implied {
        meta.includes_implied = Some(includes_implied);
    } else if edits.clear_includes_implied {
        meta.includes_implied = None;
    }
}

// --- `model.toml` projects --------------------------------------------
//
// The edits go into the existing document in place.
//
// The flags are named after `.project.json`'s fields, and three are spelled
// differently in a manifest: `maintainer`/`maintainers`, `topic`/`topics`,
// and `website`, which is one key of a `[project.urls]` table. Writing the
// json spelling would be silent: an unknown key is only a warning, and the
// value reads back as absent.
//
// A manifest also validates what the json documents do not: a maintainer
// without an email, or a name or publisher the identifier rule refuses,
// would make a `model.toml` unreadable. Those are refused here with the
// reader's own parsers. The version and the license are already parsed by
// the argument parser.
//
// Lists go through the in-place array editors so `--add-*` and
// `--remove-*` keep the comments inside them.

fn edit_toml(document: &mut DocumentMut, edits: &EditArgs) -> Result<()> {
    if let Some(name) = &edits.name {
        IndexName::parse(name.as_str()).map_err(|e| anyhow::anyhow!("`{}`: {e}", e.value))?;
        edit::set_project_key(document, ProjectKey::Name, name.as_str().into())?;
    }
    if let Some(publisher) = &edits.publisher {
        IndexPublisher::parse(publisher.as_str())
            .map_err(|e| anyhow::anyhow!("`{}`: {e}", e.value))?;
        edit::set_project_key(document, ProjectKey::Publisher, publisher.as_str().into())?;
    }
    if let Some(version) = &edits.version {
        edit::set_project_key(document, ProjectKey::Version, version.to_string().into())?;
    }
    if let Some(description) = &edits.description {
        edit::set_project_key(
            document,
            ProjectKey::Description,
            description.as_str().into(),
        )?;
    } else if edits.clear_description {
        edit::clear_project_key(document, ProjectKey::Description)?;
    }
    if let Some(license) = &edits.license {
        edit::set_project_key(document, ProjectKey::License, license.to_string().into())?;
    } else if edits.clear_license {
        edit::clear_project_key(document, ProjectKey::License)?;
    }
    // `urls` is a table, so this is not a `[project]` key
    if let Some(website) = &edits.website {
        edit::set_url_key(document, "website", website.as_str())?;
    } else if edits.clear_website {
        edit::clear_url_key(document, "website")?;
    }
    for value in &edits.add_maintainer {
        Maintainer::parse(value)?;
    }
    edit_toml_list(
        document,
        "maintainer",
        ListKey::Maintainers,
        edits.clear_maintainers,
        &edits.remove_maintainer,
        &edits.add_maintainer,
    )?;
    edit_toml_list(
        document,
        "topic",
        ListKey::Topics,
        edits.clear_topics,
        &edits.remove_topic,
        &edits.add_topic,
    )?;
    // `root` first, so that a metamodel switch in the same edit sees it
    if let Some(root) = &edits.root {
        edit::set_project_key(document, ProjectKey::Root, root.as_str().into())?;
    } else if edits.clear_root {
        edit::clear_project_key(document, ProjectKey::Root)?;
    }
    let metamodel = match (edits.metamodel, &edits.custom_metamodel) {
        (Some(kind), _) => Some(omg_metamodel(kind, edits)),
        // A `model.toml` accepts only recognized metamodels.
        (None, Some(metamodel)) => Some(crate::commands::check_metamodel(metamodel)?),
        (None, None) => None,
    };
    if let Some(metamodel) = metamodel {
        let value = metamodel.to_string();
        // A switch is refused when the target schema's required key is
        // missing, and warns when a key is left that the target does not use.
        let has_root = document
            .get("project")
            .and_then(|project| project.get("root"))
            .is_some();
        match metamodel.family {
            MetamodelFamily::Syside if !has_root => {
                bail!(
                    "changing the metamodel to `{value}` requires `root`; `model.toml` does not \
                     have it\n\
                    {SP:>8} pass `--root <PATH>` with it",
                    SP = ' ',
                );
            }
            MetamodelFamily::Omg if has_root => {
                log::warn!(
                    "`root` is only supported by the Syside metamodel and will be ignored under \
                     `{value}`; `sysand edit --clear-root` removes it"
                );
            }
            _ => {}
        }
        edit::set_project_key(document, ProjectKey::Metamodel, value.into())?;
    } else if edits.clear_metamodel {
        // A `model.toml` without a metamodel is unreadable; a json project
        // has no such rule and keeps the flag.
        bail!(
            "`metamodel` cannot be cleared from a `model.toml` project:\n\
            {SP:>8} every project declares one; `--metamodel` changes it",
            SP = ' ',
        );
    }

    Ok(())
}

/// [`edit_list`] on the `[project]` list `key`: clears it (if `clear`),
/// then removes every entry equal to one of `remove`, then appends `add`.
/// A value to remove that is not in the list is an error.
fn edit_toml_list(
    document: &mut DocumentMut,
    kind: &str,
    key: ListKey,
    clear: bool,
    remove: &[String],
    add: &[String],
) -> Result<()> {
    if clear {
        edit::clear_project_key(document, key.into())?;
    }
    for value in remove {
        let entries: Vec<String> = document
            .get("project")
            .and_then(|project| project.get(key.as_str()))
            .and_then(|list| list.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut found = false;
        // Removed from the end, so the indexes of the rest stay valid
        for (at, entry) in entries.iter().enumerate().rev() {
            if entry == value {
                edit::remove_list_entry(document, key, at)?;
                found = true;
            }
        }
        if !found {
            bail!("project has no {kind} `{value}`");
        }
    }
    if !add.is_empty() {
        let present = document
            .get("project")
            .and_then(|project| project.get(key.as_str()))
            .is_some_and(|list| list.as_array().is_some());
        if present {
            for value in add {
                edit::push_list_entry(document, key, value)?;
            }
        } else {
            let list: Array = add
                .iter()
                .map(|value| Value::from(value.as_str()))
                .collect();
            edit::set_project_key(document, key.into(), Value::Array(list))?;
        }
    }
    Ok(())
}

/// The OMG metamodel of `kind` that `--metamodel` names, at the release
/// `--metamodel-release-custom` or `--metamodel-release` gives.
fn omg_metamodel(kind: MetamodelKindCli, edits: &EditArgs) -> KnownMetamodel {
    let release = edits
        .metamodel_release_custom
        .clone()
        .unwrap_or_else(|| edits.metamodel_release.into());
    KnownMetamodel::omg(kind.into(), release)
}
