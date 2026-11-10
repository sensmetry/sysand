// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Writing a `model.toml`.
//!
//! The job divides in two. Files a command *edits* (`sysand add`,
//! `sysand remove`, `sysand include`, `sysand exclude`, the
//! `set`/`clear`/`add`/`remove` verbs of `sysand info`) are changed in
//! place, preserving comments, key order and formatting. Files a command
//! *generates* (`sysand init`, `sysand new`, `sysand migrate`) are
//! rendered in a fixed layout with no comments.
//!
//! [`render_new`] produces the fixed layout. What it returns is a
//! [`DocumentMut`], which is also what the in-place editors operate on.
//!
//! **[`render_new`] canonicalizes.** A version constraint written `1.0`
//! comes back as `^1.0`, and an SPDX expression and an IRI are rendered
//! through their `Display`. An editor therefore works on the document
//! parsed from the file and never re-renders it.
//!
//! Like the rest of `manifest/`, this module is free of Cargo features.

use serde::Serialize as _;
use thiserror::Error;
use toml_edit::{ArrayOfTables, Decor, Item, Table, value};

/// The document type every writer works on, and the two value types a
/// caller needs to build one.
///
/// Re-exported so that a crate that stores a manifest can name it without
/// depending on `toml_edit`.
pub use toml_edit::{Array, DocumentMut, Value};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, NormalizedIdentity, OMG_PUBLISHER, PathProblem, Sources, SourcesError,
    normalized_identity,
};
use crate::manifest::diagnostics::Diagnostic;
use crate::manifest::inherit::{Inherited, MetaProperty};
use crate::manifest::project::{PROJECT_KEYS, ProjectTable, URL_KEYS, Urls};
use crate::manifest::spans::Source;
use crate::manifest::usage::{MemberUsage, UsageRaw};
use crate::manifest::workspace_table::{GROUP_KEYS, GroupProperties, WorkspaceTable};
use crate::manifest::{ModelToml, SUPPORTED_MANIFEST_VERSION};
use crate::model::{IndexName, IndexPublisher, is_omg_stdlib_iri};
use crate::project::utils::ProjectDeserializationError;

/// Render a manifest as a freshly generated file.
///
/// Layout: `manifest-version` first, then `[project]` with its keys in
/// the order [`PROJECT_KEYS`] gives, except `urls`, written as a
/// `[project.urls]` table after the scalar keys, as TOML requires. `sources`
/// is a multi-line array with one file per line and a trailing comma (an
/// empty one as `sources = []`), then `[[inline-usages]]` tables follow in
/// manifest order. A workspace file has `manifest-version` and then
/// `[workspace]`.
///
/// A key the table lacks is left out.
pub fn render_new(model: &ModelToml) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["manifest-version"] = value(SUPPORTED_MANIFEST_VERSION.to_string());
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
        ModelToml::Workspace(table) => {
            doc["workspace"] = Item::Table(workspace_table(table));
        }
    }
    doc
}

/// The `[project]` table, in the order `PROJECT_KEYS` gives.
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
                Value::from(value.to_string())
            }),
            "maintainers" => optional(&mut table, key, project.maintainers.as_ref(), |list| {
                Value::Array(list.iter().map(ToString::to_string).collect())
            }),
            "stdlib" => {
                if let Some(stdlib) = &project.stdlib {
                    table[key] = value(stdlib.to_string());
                }
            }
            // Written back as it was read.
            "root" => {
                if let Some(root) = &project.root {
                    table[key] = Item::Value(
                        root.serialize(toml_edit::ser::ValueSerializer::new())
                            .expect("a TOML value serializes as a TOML value"),
                    );
                }
            }
            "urls" => urls(&mut table, key, project.urls.as_ref()),
            // Not written.
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

/// An inheritance marker: `<key>.workspace = true` inherits from the
/// default group, `<key>.workspace = "group-name"` from a named one.
///
/// A *dotted* table, so it cannot be written by indexing:
/// `table["version"]["workspace"] = value(true)` builds an ordinary
/// sub-table and emits a `[project.version]` header, which parses as
/// something else.
fn marker(inherited: &Inherited) -> Table {
    let mut table = Table::new();
    table.set_dotted(true);
    table["workspace"] = value(match &inherited.from {
        InheritFrom::DefaultGroup => Value::from(true),
        InheritFrom::Group(group) => Value::from(group.as_str()),
    });
    table
}

/// `urls` is written as a `[project.urls]` table after the scalar keys,
/// as TOML requires. An inherited `urls` has no table to write: the
/// marker takes the key's place among the scalars.
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

/// `sources` as a multi-line array with one file per line and a trailing
/// comma (an empty one as `sources = []`).
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
/// `UsageRaw`'s `serde` attributes decide which keys an entry writes and
/// what they are called.
fn usage_table(raw: UsageRaw) -> Table {
    toml_edit::ser::to_document(&raw)
        .expect("a usage entry serializes")
        .as_table()
        .clone()
}

/// The `[workspace]` table.
///
/// Order inside the table: `members`, `exclude`, the default group's keys
/// in `GROUP_KEYS` order, the default group's
/// `[[workspace.inline-usages]]`, then each named group.
fn workspace_table(workspace: &WorkspaceTable) -> Table {
    let mut table = Table::new();
    if !workspace.members.is_empty() {
        // `--member <path>` matches an entry literally, so the entry is
        // written back as it was written.
        table["members"] = value(
            workspace
                .members
                .iter()
                .map(|member| member.raw.as_str())
                .collect::<Array>(),
        );
    }
    if !workspace.exclude.is_empty() {
        table["exclude"] = value(
            workspace
                .exclude
                .iter()
                .map(|member| member.raw.as_str())
                .collect::<Array>(),
        );
    }
    group(&mut table, &workspace.default_group);

    if !workspace.groups.is_empty() {
        let mut groups = Table::new();
        // Only the children get headers.
        groups.set_implicit(true);
        for (name, properties) in &workspace.groups {
            let mut written = Table::new();
            group(&mut written, properties);
            groups.insert(name, Item::Table(written));
        }
        table.insert("group", Item::Table(groups));
    }
    table
}

/// The keys one group gives, in `GROUP_KEYS` order.
///
/// A group's values are plain, so there are no markers here.
fn group(table: &mut Table, properties: &GroupProperties) {
    for key in GROUP_KEYS {
        match *key {
            "publisher" => {
                if let Some(publisher) = &properties.publisher {
                    table[key] = value(IndexPublisher::as_str(publisher));
                }
            }
            "version" => {
                if let Some(version) = &properties.version {
                    table[key] = value(version.to_string());
                }
            }
            "license" => {
                if let Some(license) = &properties.license {
                    table[key] = value(license.to_string());
                }
            }
            "topics" => {
                if let Some(topics) = &properties.topics {
                    table[key] = value(topics.iter().map(String::as_str).collect::<Array>());
                }
            }
            "metamodel" => {
                if let Some(metamodel) = &properties.metamodel {
                    table[key] = value(metamodel.to_string());
                }
            }
            "maintainers" => {
                if let Some(maintainers) = &properties.maintainers {
                    table[key] = value(
                        maintainers
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Array>(),
                    );
                }
            }
            // Not allowed in a group; there is nothing to write.
            "sources" => {}
            "urls" => {
                if let Some(urls) = &properties.urls {
                    let mut written = Table::new();
                    for url_key in URL_KEYS {
                        let iri = match *url_key {
                            "website" => urls.website.as_ref(),
                            "documentation" => urls.documentation.as_ref(),
                            "repository" => urls.repository.as_ref(),
                            _ => None,
                        };
                        if let Some(iri) = iri {
                            written[url_key] = value(iri.as_str());
                        }
                    }
                    table.insert("urls", Item::Table(written));
                }
            }
            "inline-usages" if !properties.inline_usages.is_empty() => {
                let mut usages = ArrayOfTables::new();
                for usage in &properties.inline_usages {
                    usages.push(usage_table(UsageRaw::from(usage.clone())));
                }
                table.insert(key, Item::ArrayOfTables(usages));
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// Editing an existing document.
//
// `toml_edit` does not preserve comments for free. A value replaced the
// obvious way loses the comment after it, because the comment lives in
// the `Decor` of the value that was replaced; a key replaced by a dotted
// table loses the comment above it as well, because `Table::insert`
// moves decor.
// ---------------------------------------------------------------------

/// Why an edit was refused.
///
/// Every variant is a fact about the document, not about a command's flags.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EditError {
    #[error("the document has no `[project]` table")]
    NotAProject,
    #[error("the document has no `[workspace]` table")]
    NotAWorkspace,
    #[error("`{0}` is not a `[project]` key")]
    UnknownKey(String),
    #[error("`[project]` has no `sources` array; add one")]
    NoSources,
    #[error(transparent)]
    Sources(#[from] SourcesError),
    #[error("`{0}` is not listed in `sources`")]
    SourceNotListed(Utf8UnixPathBuf),
    /// `sources` entries are relative to the project root, and `include`
    /// cannot convert a path that leaves the project.
    #[error("`{0}` is outside the project, so it cannot be listed in `sources`")]
    SourceOutsideProject(String),
    #[error("`{0}` is already declared")]
    DuplicateUsage(NormalizedIdentity),
    #[error("`{0}` is not declared")]
    UsageNotFound(NormalizedIdentity),
    #[error("`{0}` is not an index usage and has no version")]
    NotAnIndexUsage(NormalizedIdentity),
    #[error("`{0}` is already a member")]
    DuplicateMember(Utf8UnixPathBuf),
    #[error("`{key}` has {len} entries; there is none at {index}")]
    NoSuchEntry {
        key: String,
        index: usize,
        len: usize,
    },
    #[error("`{0}` is not a list")]
    NotAList(String),
    #[error(transparent)]
    MemberPath(#[from] crate::utils::NormalizeRelativeError),
}

/// Parse a `model.toml` for editing, after checking that it is one this
/// Sysand can read.
///
/// [`open`] is a bare parse. Here the full read runs first, for its
/// diagnostics alone, and the document is handed back only when there are
/// none.
pub fn validated(file: &str, text: &str) -> Result<DocumentMut, ProjectDeserializationError> {
    crate::manifest::ModelToml::parse(file, text)
        .map_err(|report| ProjectDeserializationError::invalid(file, report))?;
    open(file, text).map_err(|problem| {
        ProjectDeserializationError::invalid(
            file,
            crate::manifest::diagnostics::Diagnostics::from_iter([problem]),
        )
    })
}

/// Parse a `model.toml` for editing. The failure is the reader's.
pub fn open(file: &str, text: &str) -> Result<DocumentMut, Diagnostic> {
    Source::new(file, text).parse_mut()
}

fn project_mut(doc: &mut DocumentMut) -> Result<&mut Table, EditError> {
    doc.get_mut("project")
        .and_then(Item::as_table_mut)
        .ok_or(EditError::NotAProject)
}

/// The comments around a key that is about to be replaced: the one on
/// the line above it, and the one after its value.
///
/// A scalar keeps the first on its own key and the second on its value; a
/// dotted marker (`version.workspace = true`) keeps both on the inner
/// `workspace` key and value, and its outer key carries nothing. Both are
/// put back on the scalar that replaces it.
#[derive(Default)]
struct KeyComments {
    above: Option<String>,
    after: Option<String>,
}

impl KeyComments {
    fn read(table: &Table, key: &str) -> Self {
        let outer = table
            .key(key)
            .and_then(|key| key.leaf_decor().prefix())
            .and_then(|prefix| prefix.as_str())
            .map(str::to_owned);
        match table.get(key) {
            Some(Item::Value(value)) => Self {
                above: outer,
                after: decor_suffix(value.decor()),
            },
            // A dotted table: the comments are on its one inner key.
            Some(Item::Table(inner)) if inner.is_dotted() => {
                let (above, after) = inner.iter().next().map_or_default(|(name, _)| {
                    (
                        inner
                            .key(name)
                            .and_then(|key| key.leaf_decor().prefix())
                            .and_then(|prefix| prefix.as_str())
                            .map(str::to_owned),
                        inner
                            .get(name)
                            .and_then(Item::as_value)
                            .and_then(|value| decor_suffix(value.decor())),
                    )
                });
                Self { above, after }
            }
            _ => Self {
                above: outer,
                after: None,
            },
        }
    }

    /// Put them back on a scalar.
    fn onto_value(self, table: &mut Table, key: &str, mut value: Value) {
        if let Some(after) = self.after {
            value.decor_mut().set_suffix(after);
        }
        table.insert(key, Item::Value(value));
        if let Some(above) = self.above
            && let Some(mut key) = table.key_mut(key)
        {
            key.leaf_decor_mut().set_prefix(above);
        }
    }
}

/// The key a marker is written under.
const MARKER: &str = "workspace";

fn decor_suffix(decor: &Decor) -> Option<String> {
    decor
        .suffix()
        .and_then(|suffix| suffix.as_str())
        .filter(|suffix| !suffix.trim().is_empty())
        .map(str::to_owned)
}

/// Set a `[project]` key to a value, keeping the comments around it.
///
/// A key that is not there is appended after the table's existing values,
/// inside `[project]` and before the first sub-table header, not in the
/// order of a generated file: moving existing lines would drag a comment
/// away from what it describes.
pub fn set_project_key(
    doc: &mut DocumentMut,
    key: ProjectKey,
    value: Value,
) -> Result<(), EditError> {
    let key = key.as_str();
    let project = project_mut(doc)?;
    KeyComments::read(project, key).onto_value(project, key, value);
    Ok(())
}

/// Remove a `[project]` key. Answers whether it was there.
pub fn clear_project_key(doc: &mut DocumentMut, key: ProjectKey) -> Result<bool, EditError> {
    Ok(project_mut(doc)?.remove(key.as_str()).is_some())
}

/// A `[project]` key that [`set_project_key`] and [`clear_project_key`]
/// edit as a whole value.
///
/// `sources` has rules of its own and goes through [`include_source`];
/// `urls` is a table and goes through [`set_url_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectKey {
    Publisher,
    Name,
    Version,
    License,
    Description,
    Topics,
    Metamodel,
    Maintainers,
    Stdlib,
    Root,
}

impl ProjectKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Name => "name",
            Self::Version => "version",
            Self::License => "license",
            Self::Description => "description",
            Self::Topics => "topics",
            Self::Metamodel => "metamodel",
            Self::Maintainers => "maintainers",
            Self::Stdlib => "stdlib",
            Self::Root => "root",
        }
    }
}

/// A list-valued `[project]` key, which [`push_list_entry`] and
/// [`remove_list_entry`] edit entry by entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Topics,
    Maintainers,
}

impl ListKey {
    pub fn as_str(self) -> &'static str {
        ProjectKey::from(self).as_str()
    }
}

impl From<ListKey> for ProjectKey {
    fn from(key: ListKey) -> Self {
        match key {
            ListKey::Topics => Self::Topics,
            ListKey::Maintainers => Self::Maintainers,
        }
    }
}

/// Set one key of `[project.urls]`.
///
/// `urls` is written in three shapes: a `[project.urls]` header, an
/// inline `urls = { .. }`, and dotted `urls.website = ..` keys. An
/// existing table is edited in whichever shape it has; a table that is
/// not there is created as a header. Indexing into the document would
/// create the inline form instead.
pub fn set_url_key(doc: &mut DocumentMut, key: &str, url: &str) -> Result<(), EditError> {
    check_url_key(key)?;
    let project = project_mut(doc)?;
    match project.get_mut("urls") {
        Some(Item::Value(Value::InlineTable(inline))) => {
            inline.insert(key, Value::from(url));
        }
        Some(Item::Table(table)) => {
            let comments = KeyComments::read(table, key);
            comments.onto_value(table, key, Value::from(url));
        }
        _ => {
            let mut urls = Table::new();
            urls[key] = value(url);
            project.insert("urls", Item::Table(urls));
        }
    }
    Ok(())
}

/// Remove one key of `[project.urls]`. Answers whether it was there.
///
/// The table is left even when the last key goes.
pub fn clear_url_key(doc: &mut DocumentMut, key: &str) -> Result<bool, EditError> {
    check_url_key(key)?;
    let project = project_mut(doc)?;
    Ok(match project.get_mut("urls") {
        Some(Item::Value(Value::InlineTable(inline))) => inline.remove(key).is_some(),
        Some(Item::Table(table)) => table.remove(key).is_some(),
        _ => false,
    })
}

fn check_url_key(key: &str) -> Result<(), EditError> {
    if URL_KEYS.contains(&key) {
        Ok(())
    } else {
        Err(EditError::UnknownKey(key.to_owned()))
    }
}

/// Append a path to `sources`, normalized as on read, rejecting a
/// duplicate.
///
/// The whole array is re-validated, so an array that was already invalid
/// fails naming an entry the caller did not touch.
pub fn include_source(doc: &mut DocumentMut, path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    let wanted = strict_entry(path)?;
    let array = sources_mut(doc)?;
    let mut candidate = entries(array);
    candidate.push(wanted.clone());
    Sources::new(candidate)?;

    push_entry(array, wanted.as_str());
    Ok(wanted)
}

/// Convert a path into the strict form `sources` holds: `./a.sysml`
/// becomes `a.sysml`, and `../a.sysml` is
/// [`EditError::SourceOutsideProject`].
fn strict_entry(path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    crate::utils::normalize_relative(path).map_err(|error| match error {
        crate::utils::NormalizeRelativeError::EscapesRoot { path: _ }
        | crate::utils::NormalizeRelativeError::IsRoot { path: _ } => {
            EditError::SourceOutsideProject(path.to_owned())
        }
        crate::utils::NormalizeRelativeError::Absolute { path: _ } => {
            EditError::Sources(SourcesError::path(PathProblem::Absolute, path))
        }
        crate::utils::NormalizeRelativeError::ContainsBackslash { path: _ } => {
            EditError::Sources(SourcesError::path(PathProblem::Backslash, path))
        }
        crate::utils::NormalizeRelativeError::ContainsNull { path: _ } => {
            EditError::Sources(SourcesError::path(PathProblem::Null, path))
        }
    })
}

/// Remove a path from `sources`. Matching is by normalized path, so
/// `./a.sysml` removes an entry written as `a.sysml`.
pub fn exclude_source(doc: &mut DocumentMut, path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    // Held to the same rules as an `include` argument, so `README.md` is
    // reported as not a model file.
    let wanted = strict_entry(path)?;
    let wanted = Sources::new(vec![wanted])?
        .as_slice()
        .first()
        .expect("one path in, one path out")
        .clone();

    let array = sources_mut(doc)?;
    let at = entries(array)
        .iter()
        .position(|entry| *entry == wanted)
        .ok_or_else(|| EditError::SourceNotListed(wanted.clone()))?;

    remove_entry(array, at);
    Ok(wanted)
}

/// Append one entry to a list-valued `[project]` key.
///
/// Editing the array in place keeps the comments inside it, which
/// replacing it with [`set_project_key`] would lose.
pub fn push_list_entry(doc: &mut DocumentMut, key: ListKey, value: &str) -> Result<(), EditError> {
    push_entry(list_mut(doc, key)?, value);
    Ok(())
}

/// Remove the entry at `at` from a list-valued `[project]` key.
///
/// An out of range index is an error.
pub fn remove_list_entry(doc: &mut DocumentMut, key: ListKey, at: usize) -> Result<(), EditError> {
    let array = list_mut(doc, key)?;
    if at >= array.len() {
        return Err(EditError::NoSuchEntry {
            key: key.as_str().to_owned(),
            index: at,
            len: array.len(),
        });
    }
    remove_entry(array, at);
    Ok(())
}

/// The array a list-valued `[project]` key holds.
///
/// A key that is absent is created empty; a key holding something that is
/// not an array is a manifest problem, not an edit.
fn list_mut(doc: &mut DocumentMut, key: ListKey) -> Result<&mut Array, EditError> {
    let key = key.as_str();
    let project = project_mut(doc)?;
    if project.get(key).is_none() {
        project[key] = value(Array::new());
    }
    project
        .get_mut(key)
        .and_then(Item::as_array_mut)
        .ok_or_else(|| EditError::NotAList(key.to_owned()))
}

fn sources_mut(doc: &mut DocumentMut) -> Result<&mut Array, EditError> {
    project_mut(doc)?
        .get_mut("sources")
        .and_then(Item::as_array_mut)
        .ok_or(EditError::NoSources)
}

fn entries(array: &Array) -> Vec<Utf8UnixPathBuf> {
    array
        .iter()
        .filter_map(|item| item.as_str())
        .map(Utf8UnixPathBuf::from)
        .collect()
}

/// Append one entry, keeping the array's shape.
///
/// `Array::push` gives the new entry no decor at all, so on a multi-line
/// array it lands on the previous entry's line. Empty, single-line and
/// multi-line arrays are told apart.
fn push_entry(array: &mut Array, text: &str) {
    let multi_line = array
        .iter()
        .filter_map(|item| item.decor().prefix().and_then(|prefix| prefix.as_str()))
        .any(|prefix| prefix.contains('\n'))
        || trailing(array).contains('\n')
        || array
            .iter()
            .filter_map(|item| item.decor().suffix().and_then(|suffix| suffix.as_str()))
            .any(|suffix| suffix.contains('\n'));

    if array.is_empty() {
        array.push(text);
        set_prefix(array, 0, "");
        return;
    }
    if !multi_line {
        array.push(text);
        set_prefix(array, array.len() - 1, " ");
        return;
    }

    // An array written without a trailing comma keeps the newline before
    // `]` in the last entry's suffix, where a push would put the comma
    // after it (`"A.kerml"\n,`). Move it into `trailing`.
    let last = array.len() - 1;
    let suffix = array
        .get(last)
        .and_then(|item| item.decor().suffix().and_then(|suffix| suffix.as_str()))
        .unwrap_or_default()
        .to_owned();
    if suffix.contains('\n') {
        let moved = format!("{suffix}{}", trailing(array));
        if let Some(item) = array.get_mut(last) {
            item.decor_mut().set_suffix("");
        }
        array.set_trailing(moved);
        array.set_trailing_comma(true);
    }

    // What sits in `trailing` up to its first newline is on the last
    // entry's line; the rest is the closing indentation.
    let trailing = trailing(array);
    let (head, tail) = match trailing.split_once('\n') {
        Some((head, tail)) => (head.to_owned(), tail.to_owned()),
        None => (trailing, String::new()),
    };
    let indent = indent_of(array);

    array.push(text);
    set_prefix(array, array.len() - 1, &format!("{head}\n{indent}"));
    array.set_trailing(format!("\n{tail}"));
    array.set_trailing_comma(true);
}

/// Remove one entry, leaving the comments that describe the others.
///
/// A comment written at the end of one entry's line is stored at the
/// front of the next entry's prefix. So the successor inherits the
/// removed entry's prefix head (everything through its first newline,
/// which was on the line above) and keeps its own prefix tail. Only what
/// was physically on the removed entry's line goes.
///
/// The last entry has no successor, so the array's trailing plays that
/// part.
fn remove_entry(array: &mut Array, at: usize) {
    let removed = prefix_of(array, at);
    let head = match removed.split_once('\n') {
        Some((head, _)) => format!("{head}\n"),
        // Nothing above: whatever is here was on the previous entry's
        // line, and is kept only if it is a comment.
        None if removed.contains('#') => removed,
        None => String::new(),
    };

    if at + 1 < array.len() {
        let successor = prefix_of(array, at + 1);
        let tail = successor
            .split_once('\n')
            .map_or(String::new(), |(_, tail)| tail.to_owned());
        array.remove(at);
        set_prefix(array, at, &format!("{head}{tail}"));
    } else {
        let tail = trailing(array)
            .split_once('\n')
            .map_or(String::new(), |(_, tail)| format!("\n{tail}"));
        array.remove(at);
        array.set_trailing(format!("{head}{tail}").replace("\n\n", "\n"));
    }
}

fn prefix_of(array: &Array, at: usize) -> String {
    array
        .get(at)
        .and_then(|item| item.decor().prefix().and_then(|prefix| prefix.as_str()))
        .unwrap_or_default()
        .to_owned()
}

fn trailing(array: &Array) -> String {
    array.trailing().as_str().unwrap_or_default().to_owned()
}

/// The indentation the array's entries are written at.
fn indent_of(array: &Array) -> String {
    array
        .iter()
        .filter_map(|item| item.decor().prefix().and_then(|prefix| prefix.as_str()))
        .filter_map(|prefix| {
            prefix
                .rsplit_once('\n')
                .map(|(_, indent)| indent.to_owned())
        })
        .last()
        .unwrap_or_else(|| "    ".to_owned())
}

fn set_prefix(array: &mut Array, at: usize, prefix: &str) {
    if let Some(item) = array.get_mut(at) {
        item.decor_mut().set_prefix(prefix);
    }
}

/// Append one `[[inline-usages]]` entry.
///
/// Duplicates are compared after normalization, so an entry naming the
/// same project under a different spelling is refused.
///
/// The entry lands after the last one and before whatever trails the
/// file, which `toml_edit` keeps where it is.
pub fn push_usage(doc: &mut DocumentMut, usage: &MemberUsage) -> Result<(), EditError> {
    let wanted = usage.identity();
    if find_usage(doc, &wanted).is_some() {
        return Err(EditError::DuplicateUsage(wanted));
    }
    let entry = usage_table(UsageRaw::from(usage.clone()));
    if let Some(usages) = doc.get_mut(USAGES).and_then(Item::as_array_of_tables_mut) {
        usages.push(entry);
    } else {
        let mut usages = ArrayOfTables::new();
        usages.push(entry);
        doc[USAGES] = Item::ArrayOfTables(usages);
    }
    Ok(())
}

/// Remove the entry that declares a project.
///
/// The comment introducing an entry is that table's own prefix and goes
/// with it.
pub fn remove_usage(doc: &mut DocumentMut, wanted: &NormalizedIdentity) -> Result<(), EditError> {
    let at = find_usage(doc, wanted).ok_or_else(|| EditError::UsageNotFound(wanted.clone()))?;
    doc.get_mut(USAGES)
        .and_then(Item::as_array_of_tables_mut)
        .expect("the entry was just found")
        .remove(at);
    Ok(())
}

/// The constraint of the entry declaring `wanted`, if it is there.
///
/// `Some(None)` is an entry with no `version` key: a `dir`, `kpar-path`,
/// `kpar-url` or opt-in usage. `None` is no entry at all.
pub fn usage_constraint(doc: &DocumentMut, wanted: &NormalizedIdentity) -> Option<Option<String>> {
    let at = find_usage(doc, wanted)?;
    let entry = doc.get(USAGES)?.as_array_of_tables()?.get(at)?;
    Some(
        entry
            .get("version")
            .and_then(Item::as_str)
            .map(str::to_owned),
    )
}

/// Set the version constraint of an index usage.
///
/// Only an index usage has one: a `dir`, `kpar-path` or `kpar-url` entry
/// names its target directly, and an opt-in takes the group's entry
/// whole. Both are refused with the same error.
pub fn set_usage_version(
    doc: &mut DocumentMut,
    wanted: &NormalizedIdentity,
    constraint: &semver::VersionReq,
) -> Result<(), EditError> {
    let at = find_usage(doc, wanted).ok_or_else(|| EditError::UsageNotFound(wanted.clone()))?;
    let entry = doc
        .get_mut(USAGES)
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|usages| usages.get_mut(at))
        .expect("the entry was just found");

    if ["dir", "kpar-path", "kpar-url", "git", MARKER]
        .iter()
        .any(|key| entry.contains_key(key))
    {
        return Err(EditError::NotAnIndexUsage(wanted.clone()));
    }

    let comments = KeyComments::read(entry, "version");
    comments.onto_value(entry, "version", Value::from(constraint.to_string()));
    Ok(())
}

/// Add a directory to a workspace's `members`.
///
/// The path is written as [`crate::utils::normalize_relative`] converts
/// it: `./app/` is written `app`.
pub fn push_member(doc: &mut DocumentMut, path: &str) -> Result<(), EditError> {
    let workspace = doc
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
        .ok_or(EditError::NotAWorkspace)?;

    let normalized = crate::utils::normalize_relative(path)?;
    if !workspace.get("members").is_some_and(Item::is_array) {
        workspace["members"] = value(Array::new());
    }
    let members = workspace["members"]
        .as_array_mut()
        .expect("written as an array just above");
    if members
        .iter()
        .filter_map(|item| item.as_str())
        .any(|listed| listed == normalized.as_str())
    {
        return Err(EditError::DuplicateMember(normalized));
    }
    push_entry(members, normalized.as_str());
    Ok(())
}

/// The top-level array a project's usages are written in.
const USAGES: &str = "inline-usages";

/// Where the entry declaring `wanted` is, if it is there.
///
/// An entry whose `publisher` or `name` is absent, of the wrong type, or
/// not a valid identifier yields nothing: it is neither matched nor in the
/// way of the entry that is.
fn find_usage(doc: &DocumentMut, wanted: &NormalizedIdentity) -> Option<usize> {
    doc.get(USAGES)
        .and_then(Item::as_array_of_tables)?
        .iter()
        .position(|entry| entry_identity(entry).as_ref() == Some(wanted))
}

/// A standard-library entry declares no `publisher` and resolves under
/// `omg`, so it is reached by `(omg, <name>)` like any other identity.
///
/// Two spellings reach `omg`: the `kpar-url` at a standard-library IRI,
/// and an opt-in to a group entry that is one, which carries a
/// `workspace` marker and a `name`. The marker's value is not read here:
/// an entry whose marker is invalid still declares that project.
fn entry_identity(entry: &Table) -> Option<NormalizedIdentity> {
    let field = |key| entry.get(key).and_then(Item::as_str);
    let name = IndexName::parse(field("name")?).ok()?;
    let publisher = match field("publisher") {
        Some(raw) => IndexPublisher::parse(raw).ok()?,
        None if field("kpar-url").is_some_and(is_omg_stdlib_iri)
            || entry.get("workspace").is_some() =>
        {
            OMG_PUBLISHER.clone()
        }
        None => return None,
    };
    Some(normalized_identity(&publisher, &name))
}

#[cfg(test)]
#[path = "./edit_tests.rs"]
mod tests;
