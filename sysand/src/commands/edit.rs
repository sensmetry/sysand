// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use sysand_core::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::local_src::LocalSrcProject,
};

use crate::{
    cli::{EditArgs, Metamodel},
    commands::info::{
        get_info_or_bail, get_meta_or_bail, log_license_files_note, set_info_or_bail,
        set_meta_or_bail,
    },
};

/// Applies `edits` to `project`. Each of `.project.json` and `.meta.json` is
/// read and written (once) only if one of its fields is edited
pub fn command_edit(mut project: LocalSrcProject, edits: EditArgs) -> Result<()> {
    let (edits_info, edits_meta) = (edits_info(&edits), edits_meta(&edits));
    if !edits_info && !edits_meta {
        bail!("no edits given, see `sysand edit --help` for the fields that can be edited");
    }
    if edits_info {
        let mut info = get_info_or_bail(&project)?;
        edit_info(&edits, &mut info)?;
        set_info_or_bail(&mut project, &info)?;
    }
    if edits_meta {
        let mut meta = get_meta_or_bail(&project)?;
        edit_meta(&edits, &mut meta);
        set_meta_or_bail(&mut project, &meta)?;
    }
    if edits.license.is_some() {
        log_license_files_note();
    }
    Ok(())
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
        ..
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
        includes_derived,
        clear_includes_derived,
        includes_implied,
        clear_includes_implied,
        ..
    } = edits;
    metamodel.is_some()
        || custom_metamodel.is_some()
        || *clear_metamodel
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
/// is an error, so that typos do not go unnoticed
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

fn edit_meta(edits: &EditArgs, meta: &mut InterchangeProjectMetadataRaw) {
    if let Some(kind) = edits.metamodel {
        meta.metamodel = Some(match edits.metamodel_release_custom {
            Some(release) => format!("{kind}{release}"),
            None => Metamodel(kind, edits.metamodel_release).into(),
        });
    } else if let Some(metamodel) = &edits.custom_metamodel {
        meta.metamodel = Some(metamodel.clone());
    } else if edits.clear_metamodel {
        meta.metamodel = None;
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
