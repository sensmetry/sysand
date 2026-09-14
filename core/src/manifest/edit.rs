// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Writing a `model.toml`.
//!
//! model-toml.md "Editing the manifest" divides the job in two. Files a
//! command *edits* -- `sysand add`, `sysand remove`, `sysand include`,
//! `sysand exclude`, the `set`/`clear`/`add`/`remove` verbs of `sysand
//! info` -- are changed in place, "preserving comments, key order and
//! formatting; comments are the reason a TOML manifest exists, so no
//! writer may drop them". Files a command *generates* -- `sysand init`,
//! `sysand new`, `sysand migrate` -- are rendered "in a fixed layout,
//! without comments".
//!
//! This module is the second half: [`render_new`] produces the fixed
//! layout. "One code path serves generated and edited files" is what the
//! shared type gives -- what [`render_new`] returns is a
//! [`DocumentMut`], which is what the in-place editors operate on.
//!
//! **[`render_new`] canonicalizes.** A version constraint written `1.0`
//! comes back as `^1.0`, an SPDX expression and an IRI are rendered
//! through their `Display`. That is right for a file being generated and
//! wrong for one being edited, so an editor works on the document parsed
//! from the file and never re-renders it.
//!
//! Like the rest of `manifest/`, this module is free of Cargo features.

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value, value};

use crate::manifest::common::{InheritFrom, Sources};
use crate::manifest::inherit::{Inherited, MetaProperty};
use crate::manifest::project::{PROJECT_KEYS, ProjectTable, URL_KEYS, Urls};
use crate::manifest::usage::UsageRaw;
use crate::manifest::{ModelToml, SUPPORTED_MANIFEST_VERSION};

/// Render a manifest as a freshly generated file.
///
/// model-toml.md "Editing the manifest": "`manifest-version` first, then
/// `[project]` with its keys in the order Main project metadata
/// documents them -- except `urls`, written as a `[project.urls]` table
/// after the scalar keys, as TOML requires -- with `sources` as a
/// multi-line array with one file per line and a trailing comma (an
/// empty one as `sources = []`), then `[[inline-usages]]` tables in
/// manifest order; a workspace file has `manifest-version` and then
/// `[workspace]`."
///
/// What is written is what the manifest holds: a key the table does not
/// have is not a key with a default, and is left out.
pub fn render_new(model: &ModelToml) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["manifest-version"] = value(SUPPORTED_MANIFEST_VERSION);
    match model {
        ModelToml::Project(manifest) => {
            doc["project"] = Item::Table(project_table(&manifest.project));
            if !manifest.inline_usages.is_empty() {
                let mut usages = ArrayOfTables::new();
                for usage in &manifest.inline_usages {
                    usages.push(usage_table(UsageRaw::from(usage.clone())));
                }
                doc["inline-usages"] = Item::ArrayOfTables(usages);
            }
        }
        // The `[workspace]` body lands with its tests in the next
        // commit; the arm is here because `render_new` is total over
        // `ModelToml` and a `todo!()` on a public function's own
        // argument type is a trap for whoever calls it first.
        ModelToml::Workspace(_) => {
            doc["workspace"] = Item::Table(Table::new());
        }
    }
    doc
}

/// The `[project]` table, in the order `PROJECT_KEYS` gives.
///
/// The key list is the reader's, not a second copy of it: it already
/// declares itself to be "every key model-toml.md Main project metadata
/// documents, in the order it documents them, which is the order a
/// generated file writes". Iterating it here is what keeps the two from
/// drifting, and `every_documented_project_key_is_rendered` is what
/// catches a key added to the list that this match does not write.
fn project_table(project: &ProjectTable) -> Table {
    let mut table = Table::new();
    for key in PROJECT_KEYS {
        match *key {
            "publisher" => meta(&mut table, key, &project.publisher, |value| {
                Value::from(value.as_str())
            }),
            "name" => table[key] = value(project.name.as_str()),
            "version" => meta(&mut table, key, &project.version, |value| {
                Value::from(value.to_string())
            }),
            "sources" => {
                if let Some(sources) = &project.sources {
                    table[key] = value(sources_array(sources));
                }
            }
            "license" => optional(&mut table, key, project.license.as_ref(), |value| {
                Value::from(value.to_string())
            }),
            "description" => {
                if let Some(description) = &project.description {
                    table[key] = value(description.as_str());
                }
            }
            "topics" => optional(&mut table, key, project.topics.as_ref(), |topics| {
                Value::Array(topics.iter().map(String::as_str).collect())
            }),
            "metamodel" => optional(&mut table, key, project.metamodel.as_ref(), |value| {
                Value::from(value.as_str())
            }),
            "maintainers" => optional(&mut table, key, project.maintainers.as_ref(), |list| {
                Value::Array(list.iter().map(ToString::to_string).collect())
            }),
            "stdlib" => {
                if let Some(stdlib) = &project.stdlib {
                    table[key] = value(stdlib.to_string());
                }
            }
            // model-toml.md reserves `root` for the Syside metamodel, so
            // nothing in v1 sets it -- but a manifest that was read
            // carries it, and a writer that dropped it would lose the
            // key on every rewrite.
            "root" => {
                if project.root {
                    table[key] = value(true);
                }
            }
            "urls" => urls(&mut table, key, project.urls.as_ref()),
            // A key added to `PROJECT_KEYS` and not here. Silence rather
            // than a panic: a library that aborts on its own data is
            // worse than one that writes a key short, and the test named
            // above fails either way.
            _ => {}
        }
    }
    table
}

/// A key that always has a value, and may instead carry a marker.
fn meta<T>(
    table: &mut Table,
    key: &str,
    property: &MetaProperty<T>,
    render: impl FnOnce(&T) -> Value,
) {
    match property {
        MetaProperty::Provided(provided) => table[key] = value(render(provided)),
        MetaProperty::Inherited(inherited) => {
            table.insert(key, Item::Table(marker(inherited)));
        }
    }
}

/// The same, for a key that may also be absent.
fn optional<T>(
    table: &mut Table,
    key: &str,
    property: Option<&MetaProperty<T>>,
    render: impl FnOnce(&T) -> Value,
) {
    if let Some(property) = property {
        meta(table, key, property, render);
    }
}

/// workspaces.md "Explicit inheritance": "to inherit a key from the
/// default group, `<key>.workspace = true` is used. To inherit from a
/// non-default group, `<key>.workspace = "group-name"`."
///
/// A *dotted* table, which is why this cannot be written by indexing:
/// `table["version"]["workspace"] = value(true)` builds an ordinary
/// sub-table and emits a `[project.version]` header, which is a file
/// that parses as something else entirely.
fn marker(inherited: &Inherited) -> Table {
    let mut table = Table::new();
    table.set_dotted(true);
    table["workspace"] = value(match &inherited.from {
        InheritFrom::DefaultGroup => Value::from(true),
        InheritFrom::Group(group) => Value::from(group.as_str()),
    });
    table
}

/// model-toml.md: `urls` is "written as a `[project.urls]` table after
/// the scalar keys, as TOML requires". An *inherited* `urls` has no
/// table to write: the marker takes the key's place among the scalars.
fn urls(table: &mut Table, key: &str, property: Option<&MetaProperty<Urls>>) {
    let urls = match property {
        None => return,
        Some(MetaProperty::Inherited(inherited)) => {
            table.insert(key, Item::Table(marker(inherited)));
            return;
        }
        Some(MetaProperty::Provided(urls)) => urls,
    };

    let mut written = Table::new();
    for url_key in URL_KEYS {
        let property = match *url_key {
            "website" => urls.website.as_ref(),
            "documentation" => urls.documentation.as_ref(),
            "repository" => urls.repository.as_ref(),
            _ => None,
        };
        optional(&mut written, url_key, property, |iri| {
            Value::from(iri.as_str())
        });
    }
    table.insert(key, Item::Table(written));
}

/// model-toml.md: "`sources` as a multi-line array with one file per
/// line and a trailing comma (an empty one as `sources = []`)".
fn sources_array(sources: &Sources) -> Array {
    let mut array = Array::new();
    if sources.as_slice().is_empty() {
        return array;
    }
    for path in sources.as_slice() {
        array.push(path.as_str());
    }
    for item in array.iter_mut() {
        item.decor_mut().set_prefix("\n    ");
    }
    array.set_trailing(",\n");
    array.set_trailing_comma(false);
    array
}

/// One `[[inline-usages]]` entry.
///
/// The one place in this module that serializes. `UsageRaw` carries the
/// `serde` attributes that decide which keys an entry writes and what
/// they are called (`kpar-path`), and `From<Usage>`/`From<MemberUsage>`
/// decide what each kind contributes, so rendering through it is what
/// keeps a written entry readable by `UsageRaw::from_table`.
fn usage_table(raw: UsageRaw) -> Table {
    toml_edit::ser::to_document(&raw)
        .expect("a usage entry serializes")
        .as_table()
        .clone()
}

#[cfg(test)]
#[path = "./edit_tests.rs"]
mod tests;
