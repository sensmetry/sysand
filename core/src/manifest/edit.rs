// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Writing a `model.toml`.
//!
//! The job divides in two. Files a command *edits* (`sysand add`,
//! `sysand remove`, `sysand include`, `sysand exclude`, the
//! `set`/`clear`/`add`/`remove` verbs of `sysand info`) are changed in
//! place, preserving comments, key order and formatting, apart from an
//! array the edit changes: that array is laid out again whole, comments
//! included, in its key's [`Shape`]. Files a command *generates*
//! (`sysand init`, `sysand new`, `sysand migrate`) are rendered in a fixed
//! layout with no comments, whose arrays have the same shapes.
//!
//! Shapes: `sources`, `members`, `exclude` and an inline `inline-usages`
//! array are [`Shape::OnePerLine`]; `topics` and `maintainers` are
//! [`Shape::AsWritten`].
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

use fluent_uri::Iri;
use serde::Serialize as _;
use thiserror::Error;
use toml_edit::{ArrayOfTables, Decor, Item, Table, TableLike, value};

/// The document type every writer works on, and the two value types a
/// caller needs to build one.
///
/// Re-exported so that a crate that stores a manifest can name it without
/// depending on `toml_edit`.
pub use toml_edit::{Array, DocumentMut, Value};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, NormalizedIdentity, OMG_PUBLISHER, PathProblem, Sources, SourcesError, URL_KEYS,
    UrlsG, normalized_identity, path_problem,
};
use crate::manifest::diagnostics::Diagnostic;
use crate::manifest::inherit::{Inherited, MetaProperty};
use crate::manifest::layout::{self, Shape};
use crate::manifest::project::{PROJECT_KEYS, ProjectTable, Urls};
use crate::manifest::spans::Source;
use crate::manifest::usage::{MemberUsage, UsageRaw};
use crate::manifest::workspace_config::{GROUP_KEYS, GroupProperties, WorkspaceConfig};
use crate::manifest::{ModelToml, SUPPORTED_MANIFEST_VERSION};
use crate::model::{IndexName, IndexPublisher, is_omg_stdlib_iri};
use crate::project::utils::ProjectDeserializationError;

/// Render a manifest as a freshly generated file.
///
/// Layout: `manifest-version` first, then `[project]` with its keys in
/// the order [`PROJECT_KEYS`] gives, except `urls`, written as a
/// `[project.urls]` table after the scalar keys, as TOML requires. `sources`
/// is one file per line (an empty one is `sources = []`), then
/// `[[inline-usages]]` tables follow in manifest order. A workspace file has
/// `manifest-version` and then `[workspace]`, whose `members` and `exclude`
/// are one path per line.
///
/// A key the table lacks is left out.
pub fn render_new(model: &ModelToml) -> DocumentMut {
    let mut doc = DocumentMut::new();
    doc["manifest-version"] = value(SUPPORTED_MANIFEST_VERSION.to_string());
    match model {
        ModelToml::Project(manifest) => {
            doc["project"] = Item::Table(project_table(&manifest.project));
            if !manifest.inline_usages.is_empty() {
                doc["inline-usages"] = Item::ArrayOfTables(usages_array(&manifest.inline_usages));
            }
        }
        ModelToml::Workspace(table) => {
            doc["workspace"] = Item::Table(workspace_config(table));
        }
    }
    doc
}

/// The `[project]` table, in the order `PROJECT_KEYS` gives.
fn project_table(project: &ProjectTable) -> Table {
    let mut table = Table::new();
    for key in PROJECT_KEYS {
        match *key {
            "publisher" => meta(&mut table, key, &project.publisher, publisher_value),
            "name" => table[key] = value(project.name.as_str()),
            "version" => meta(&mut table, key, &project.version, display_value),
            "sources" => {
                if let Some(sources) = &project.sources {
                    table[key] = value(sources_array(sources));
                }
            }
            "license" => optional(&mut table, key, project.license.as_ref(), display_value),
            "description" => plain(&mut table, key, project.description.as_ref(), |text| {
                Value::from(text.as_str())
            }),
            "topics" => optional(&mut table, key, project.topics.as_ref(), |topics| {
                list_value(topics)
            }),
            "metamodel" => optional(&mut table, key, project.metamodel.as_ref(), display_value),
            "maintainers" => optional(&mut table, key, project.maintainers.as_ref(), |list| {
                list_value(list)
            }),
            "stdlib" => plain(&mut table, key, project.stdlib.as_ref(), display_value),
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

/// A key that may be absent, with no marker.
fn plain<T>(table: &mut Table, key: &str, property: Option<&T>, render: impl FnOnce(&T) -> Value) {
    if let Some(property) = property {
        table[key] = value(render(property));
    }
}

fn publisher_value(publisher: &IndexPublisher) -> Value {
    Value::from(publisher.as_str())
}

fn display_value<T: ToString>(value: &T) -> Value {
    Value::from(value.to_string())
}

fn list_value<T: ToString>(list: &[T]) -> Value {
    Value::Array(list.iter().map(ToString::to_string).collect())
}

fn iri_value(iri: &Iri<String>) -> Value {
    Value::from(iri.as_str())
}

/// A `urls` table, each present key written by `write`.
fn urls_table<T>(urls: &UrlsG<T>, write: impl Fn(&mut Table, &str, Option<&T>)) -> Table {
    let mut written = Table::new();
    for (url_key, property) in urls.entries() {
        write(&mut written, url_key, property);
    }
    written
}

/// `[[inline-usages]]` entries.
fn usages_array<U: Clone + Into<UsageRaw>>(usages: &[U]) -> ArrayOfTables {
    let mut array = ArrayOfTables::new();
    for usage in usages {
        array.push(usage_table(usage.clone().into()));
    }
    array
}

/// A key that may be absent, and may instead carry a marker.
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

    let written = urls_table(urls, |table, url_key, property| {
        optional(table, url_key, property, iri_value);
    });
    table.insert(key, Item::Table(written));
}

/// `sources`, one file per line.
fn sources_array(sources: &Sources) -> Array {
    one_per_line(sources.as_slice().iter().map(|path| path.as_str()))
}

/// An array of strings laid out one per line.
fn one_per_line<'a>(entries: impl Iterator<Item = &'a str>) -> Array {
    let mut array: Array = entries.collect();
    layout::apply(&mut array, Shape::OnePerLine);
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
fn workspace_config(workspace: &WorkspaceConfig) -> Table {
    let mut table = Table::new();
    if !workspace.members.is_empty() {
        // `--member <path>` matches an entry literally, so the entry is
        // written back as it was written.
        table["members"] = value(one_per_line(
            workspace.members.iter().map(|member| member.raw.as_str()),
        ));
    }
    if !workspace.exclude.is_empty() {
        table["exclude"] = value(one_per_line(
            workspace.exclude.iter().map(|member| member.raw.as_str()),
        ));
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
            "publisher" => plain(table, key, properties.publisher.as_ref(), publisher_value),
            "version" => plain(table, key, properties.version.as_ref(), display_value),
            "license" => plain(table, key, properties.license.as_ref(), display_value),
            "topics" => plain(table, key, properties.topics.as_ref(), |topics| {
                list_value(topics)
            }),
            "metamodel" => plain(table, key, properties.metamodel.as_ref(), display_value),
            "maintainers" => plain(table, key, properties.maintainers.as_ref(), |list| {
                list_value(list)
            }),
            // Not allowed in a group; there is nothing to write.
            "sources" => {}
            "urls" => {
                if let Some(urls) = &properties.urls {
                    let written = urls_table(urls, |table, url_key, iri| {
                        plain(table, url_key, iri, iri_value);
                    });
                    table.insert(key, Item::Table(written));
                }
            }
            "inline-usages" if !properties.inline_usages.is_empty() => {
                table.insert(
                    key,
                    Item::ArrayOfTables(usages_array(&properties.inline_usages)),
                );
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
    /// A switch to a metamodel family that names `root` meets a `sources`
    /// with entries.
    #[error(
        "`sources` lists {0} file(s), and a Syside project lists none;\n\
         exclude them first"
    )]
    SourcesListed(usize),
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
    MemberPath(#[from] crate::paths::NormalizeError),
}

/// Parse a `model.toml` for editing, after checking that it is one this
/// Sysand can read.
///
/// [`open`] is a bare parse. Here the full read runs first, for its
/// diagnostics alone, and the document is handed back only when there are
/// none.
pub fn validated(file: &str, text: &str) -> Result<DocumentMut, ProjectDeserializationError> {
    check(file, text)?;
    open(file, text).map_err(|problem| ProjectDeserializationError::invalid(file, problem))
}

/// The text of `document`, a `model.toml` to be written, after checking
/// that it is one this Sysand can read. `file` names it in messages.
pub fn checked_text(
    file: &str,
    document: &DocumentMut,
) -> Result<String, ProjectDeserializationError> {
    let text = document.to_string();
    check(file, &text)?;
    Ok(text)
}

/// Check that `text` is a `model.toml` this Sysand can read.
fn check(file: &str, text: &str) -> Result<(), ProjectDeserializationError> {
    match ModelToml::parse(file, text) {
        Ok(_) => Ok(()),
        Err(report) => Err(ProjectDeserializationError::invalid(file, report)),
    }
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
        let outer = key_prefix(table, key);
        match table.get(key) {
            Some(Item::Value(value)) => Self {
                above: outer,
                after: decor_suffix(value.decor()),
            },
            // A dotted table: the comments are on its one inner key.
            Some(Item::Table(inner)) if inner.is_dotted() => {
                let (above, after) = inner.iter().next().map_or_default(|(name, _)| {
                    (
                        key_prefix(inner, name),
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

/// What is written before a key: the comment on the line above it.
fn key_prefix(table: &Table, key: &str) -> Option<String> {
    table
        .key(key)
        .and_then(|key| key.leaf_decor().prefix())
        .and_then(|prefix| prefix.as_str())
        .map(str::to_owned)
}

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

    layout::push(array, wanted.as_str(), Shape::OnePerLine);
    Ok(wanted)
}

/// Convert a path into the strict form `sources` holds: `./a.sysml`
/// becomes `a.sysml`, and `../a.sysml` is
/// [`EditError::SourceOutsideProject`].
fn strict_entry(path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    crate::paths::normalize_relative(path).map_err(|error| {
        PathProblem::of_normalize_error(&error).map_or_else(
            || EditError::SourceOutsideProject(path.to_owned()),
            |problem| EditError::Sources(SourcesError::path(problem, path)),
        )
    })
}

/// Remove a path from `sources`. Matching is by normalized path, so
/// `./a.sysml` removes an entry written as `a.sysml`.
pub fn exclude_source(doc: &mut DocumentMut, path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    // Held to the same rules as an `include` argument, so `README.md` is
    // reported as not a model file.
    let wanted = strict_entry(path)?;
    if let Some(problem) = path_problem(wanted.as_str()) {
        return Err(EditError::Sources(SourcesError::path(
            problem,
            wanted.as_str(),
        )));
    }

    let array = sources_mut(doc)?;
    let at = entries(array)
        .iter()
        .position(|entry| *entry == wanted)
        .ok_or_else(|| EditError::SourceNotListed(wanted.clone()))?;

    layout::remove(array, at, Shape::OnePerLine);
    Ok(wanted)
}

/// Append one entry to a list-valued `[project]` key.
///
/// Editing the array in place keeps the comments inside it, which
/// replacing it with [`set_project_key`] would lose.
pub fn push_list_entry(doc: &mut DocumentMut, key: ListKey, value: &str) -> Result<(), EditError> {
    layout::push(list_mut(doc, key)?, value, Shape::AsWritten);
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
    layout::remove(array, at, Shape::AsWritten);
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

/// Remove an empty `sources`, for a switch to a metamodel family that
/// names `root` instead. A `sources` with entries is
/// [`EditError::SourcesListed`] and is kept.
pub fn remove_empty_sources(doc: &mut DocumentMut) -> Result<(), EditError> {
    let project = project_mut(doc)?;
    let listed = project
        .get("sources")
        .and_then(Item::as_array)
        .map_or(0, Array::len);
    if listed > 0 {
        return Err(EditError::SourcesListed(listed));
    }
    project.remove("sources");
    Ok(())
}

/// Write `sources = []` where the manifest has no `sources`, for a switch
/// to a metamodel family that lists its files.
pub fn add_empty_sources(doc: &mut DocumentMut) -> Result<(), EditError> {
    let project = project_mut(doc)?;
    if !project.contains_key("sources") {
        project.insert("sources", value(Array::new()));
    }
    Ok(())
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

/// Append one usage entry.
///
/// Duplicates are compared after normalization, so an entry naming the
/// same project under a different spelling is refused.
///
/// Usages written as `[[inline-usages]]` tables get one more table, after
/// the last one and before whatever trails the file, which `toml_edit`
/// keeps where it is. Usages written as an inline
/// `inline-usages = [{ .. }]` array get an inline table appended, one
/// entry per line. A manifest with no usages gets the table form.
pub fn push_usage(doc: &mut DocumentMut, usage: &MemberUsage) -> Result<(), EditError> {
    let wanted = usage.identity();
    if find_usage(doc, &wanted).is_some() {
        return Err(EditError::DuplicateUsage(wanted));
    }
    let entry = usage_table(UsageRaw::from(usage.clone()));
    match usages_mut(doc) {
        Some(UsagesMut::Tables(usages)) => usages.push(entry),
        Some(UsagesMut::Inline(usages)) => {
            layout::push(usages, entry.into_inline_table(), Shape::OnePerLine);
        }
        None => {
            let mut usages = ArrayOfTables::new();
            usages.push(entry);
            doc[USAGES] = Item::ArrayOfTables(usages);
        }
    }
    Ok(())
}

/// Remove the entry that declares a project.
///
/// The comment introducing a `[[inline-usages]]` entry is that table's
/// own prefix and goes with it.
pub fn remove_usage(doc: &mut DocumentMut, wanted: &NormalizedIdentity) -> Result<(), EditError> {
    let at = find_usage(doc, wanted).ok_or_else(|| EditError::UsageNotFound(wanted.clone()))?;
    match usages_mut(doc).expect("the entry was just found") {
        UsagesMut::Tables(usages) => {
            usages.remove(at);
        }
        UsagesMut::Inline(usages) => layout::remove(usages, at, Shape::OnePerLine),
    }
    Ok(())
}

/// Replace the entry that declares a project with `usage`, where it is.
///
/// The comments around the entry stay with it.
pub fn replace_usage(
    doc: &mut DocumentMut,
    wanted: &NormalizedIdentity,
    usage: &MemberUsage,
) -> Result<(), EditError> {
    let at = find_usage(doc, wanted).ok_or_else(|| EditError::UsageNotFound(wanted.clone()))?;
    let mut entry = usage_table(UsageRaw::from(usage.clone()));
    match usages_mut(doc).expect("the entry was just found") {
        UsagesMut::Tables(usages) => {
            let slot = usages.get_mut(at).expect("the entry was just found");
            *entry.decor_mut() = slot.decor().clone();
            *slot = entry;
        }
        UsagesMut::Inline(usages) => {
            let slot = usages
                .get_mut(at)
                .and_then(Value::as_inline_table_mut)
                .expect("the entry was just found");
            let mut inline = entry.into_inline_table();
            *inline.decor_mut() = slot.decor().clone();
            *slot = inline;
        }
    }
    Ok(())
}

/// The constraint of the entry declaring `wanted`, if it is there.
///
/// `Some(None)` is an entry with no `version` key: a `dir`, `kpar-path`,
/// `kpar-url` or opt-in usage. `None` is no entry at all.
pub fn usage_constraint(doc: &DocumentMut, wanted: &NormalizedIdentity) -> Option<Option<String>> {
    let at = find_usage(doc, wanted)?;
    let entry = usage_entries(doc).get(at).copied().flatten()?;
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
    let is_index = |entry: &dyn TableLike| {
        !["dir", "kpar-path", "kpar-url", "git", MARKER]
            .iter()
            .any(|key| entry.contains_key(key))
    };
    let constraint = Value::from(constraint.to_string());
    match usages_mut(doc).expect("the entry was just found") {
        UsagesMut::Tables(usages) => {
            let entry = usages.get_mut(at).expect("the entry was just found");
            if !is_index(entry) {
                return Err(EditError::NotAnIndexUsage(wanted.clone()));
            }
            KeyComments::read(entry, "version").onto_value(entry, "version", constraint);
        }
        UsagesMut::Inline(usages) => {
            let entry = usages
                .get_mut(at)
                .and_then(Value::as_inline_table_mut)
                .expect("the entry was just found");
            if !is_index(entry) {
                return Err(EditError::NotAnIndexUsage(wanted.clone()));
            }
            entry.insert("version", constraint);
        }
    }
    Ok(())
}

/// Add a directory to a workspace's `members`.
///
/// The path is written as [`crate::paths::normalize_relative`] converts
/// it: `./app/` is written `app`.
pub fn push_member(doc: &mut DocumentMut, path: &str) -> Result<(), EditError> {
    let workspace = doc
        .get_mut("workspace")
        .and_then(Item::as_table_mut)
        .ok_or(EditError::NotAWorkspace)?;

    let normalized = crate::paths::normalize_relative(path)?;
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
    layout::push(members, normalized.as_str(), Shape::OnePerLine);
    Ok(())
}

/// The top-level array a project's usages are written in.
const USAGES: &str = "inline-usages";

/// The usage array, in either of the forms it is written in:
/// `[[inline-usages]]` tables or an inline `inline-usages = [{ .. }]`.
enum UsagesMut<'a> {
    Tables(&'a mut ArrayOfTables),
    Inline(&'a mut Array),
}

fn usages_mut(doc: &mut DocumentMut) -> Option<UsagesMut<'_>> {
    match doc.get_mut(USAGES)? {
        Item::ArrayOfTables(usages) => Some(UsagesMut::Tables(usages)),
        Item::Value(Value::Array(usages)) => Some(UsagesMut::Inline(usages)),
        _ => None,
    }
}

/// The entries of the usage array, in either form, by position. An
/// element of an inline array that is not an inline table is `None`.
fn usage_entries(doc: &DocumentMut) -> Vec<Option<&dyn TableLike>> {
    match doc.get(USAGES) {
        Some(Item::ArrayOfTables(usages)) => usages
            .iter()
            .map(|entry| -> Option<&dyn TableLike> { Some(entry) })
            .collect(),
        Some(Item::Value(Value::Array(usages))) => usages
            .iter()
            .map(|entry| {
                entry
                    .as_inline_table()
                    .map(|entry| -> &dyn TableLike { entry })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Where the entry declaring `wanted` is, if it is there.
///
/// An entry whose `publisher` or `name` is absent, of the wrong type, or
/// not a valid identifier yields nothing: it is neither matched nor in the
/// way of the entry that is.
fn find_usage(doc: &DocumentMut, wanted: &NormalizedIdentity) -> Option<usize> {
    usage_entries(doc)
        .into_iter()
        .position(|entry| entry.and_then(entry_identity).as_ref() == Some(wanted))
}

/// A standard-library entry declares no `publisher` and resolves under
/// `omg`, so it is reached by `(omg, <name>)` like any other identity.
///
/// Two spellings reach `omg`: the `kpar-url` at a standard-library IRI,
/// and an opt-in to a group entry that is one, which carries a
/// `workspace` marker and a `name`. The marker's value is not read here:
/// an entry whose marker is invalid still declares that project.
fn entry_identity(entry: &dyn TableLike) -> Option<NormalizedIdentity> {
    let field = |key| entry.get(key).and_then(Item::as_str);
    let name = match IndexName::parse(field("name")?) {
        Ok(name) => name,
        Err(err) => {
            log::debug!("skipping an entry with an invalid `name`: {err}");
            return None;
        }
    };
    let publisher = match field("publisher") {
        // ast-grep-ignore: dot-ok
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
