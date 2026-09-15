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

use thiserror::Error;
use toml_edit::{ArrayOfTables, Decor, Item, Table, value};

/// The document type every writer works on, and the two value types a
/// caller needs to build one.
///
/// Re-exported because a crate that stores a manifest has to name it --
/// the JS binding's browser store, any project type outside this crate
/// -- and should not have to depend on `toml_edit` to do so. The
/// `ProjectMut` derive expands into the user's crate, so it would
/// otherwise require the same dependency at every derive site.
pub use toml_edit::{Array, DocumentMut, Value};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, NormalizedIdentity, Sources, SourcesError, normalized_identity,
};
use crate::manifest::diagnostics::Diagnostic;
use crate::manifest::inherit::{Inherited, MetaProperty};
use crate::manifest::project::{PROJECT_KEYS, ProjectTable, URL_KEYS, Urls};
use crate::manifest::spans::Source;
use crate::manifest::usage::{MemberUsage, UsageRaw};
use crate::manifest::workspace_table::{GROUP_KEYS, GroupProperties, WorkspaceTable};
use crate::manifest::{ModelToml, SUPPORTED_MANIFEST_VERSION};
use crate::model::{IndexName, IndexPublisher};
use crate::project::utils::ProjectDeserializationError;

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
        ModelToml::Workspace(table) => {
            doc["workspace"] = Item::Table(workspace_table(table));
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

/// The `[workspace]` table.
///
/// model-toml.md fixes the project layout in detail and says of this one
/// only that a workspace file "has `manifest-version` and then
/// `[workspace]`". The order below is therefore Sysand's own:
/// `members`, `exclude`, the default group's keys in `GROUP_KEYS` order,
/// the default group's `[[workspace.inline-usages]]`, then each named
/// group.
fn workspace_table(workspace: &WorkspaceTable) -> Table {
    let mut table = Table::new();
    if !workspace.members.is_empty() {
        // workspaces.md "Command scopes": `--member <path>` matches an
        // entry "literally", so both spellings are kept and the one that
        // was written is the one that is written back.
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
        // `[workspace.group]` itself holds nothing: only its children
        // get headers.
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
/// A group's values are plain: workspaces.md has nothing above a
/// workspace for them to be inherited from, so there are no markers
/// here.
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
                    table[key] = value(metamodel.as_str());
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
            // workspaces.md: a group may not carry `sources`. The key is
            // in `GROUP_KEYS` so that the reader can refuse it by name,
            // and there is nothing here to write.
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
// model-toml.md "Editing the manifest": every command that writes a
// `model.toml` "edits the existing document in place, preserving
// comments, key order and formatting; comments are the reason a TOML
// manifest exists, so no writer may drop them."
//
// `toml_edit` does not give that for free. A value replaced the obvious
// way loses the comment after it, because the comment lives in the
// `Decor` of the value that was replaced; a key replaced by a dotted
// table loses the comment above it as well, because `Table::insert`
// with a `&str` key throws the old key's decor away. So every editor
// below moves decor deliberately.
// ---------------------------------------------------------------------

/// workspaces.md "Common metadata": the `[project]` keys a member may
/// inherit from a workspace group.
///
/// `project.rs` decides this one `inheritable(..)` call at a time and
/// has no list to borrow. `the_inheritable_keys_are_the_ones_the_reader_
/// accepts_a_marker_for` is what keeps the two from drifting: it writes
/// a marker for every key of `PROJECT_KEYS` and checks that the reader
/// agrees with this list.
pub const INHERITABLE_KEYS: &[&str] = &[
    "publisher",
    "version",
    "license",
    "topics",
    "metamodel",
    "maintainers",
    "urls",
];

/// Why an edit was refused.
///
/// Every variant is a fact about the *document*. A refusal of a flag
/// rather than a document (`--compute-checksum` on a toml project)
/// belongs to the command that owns the flag.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum EditError {
    #[error("the document has no `[project]` table")]
    NotAProject,
    #[error("the document has no `[workspace]` table")]
    NotAWorkspace,
    #[error("`{0}` is not a `[project]` key")]
    UnknownKey(String),
    #[error("`{0}` cannot be inherited from a workspace")]
    NotInheritable(String),
    #[error("`[project]` has no `sources` array; add one")]
    NoSources,
    #[error(transparent)]
    Sources(#[from] SourcesError),
    #[error("`{0}` is not listed in `sources`")]
    SourceNotListed(Utf8UnixPathBuf),
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

/// Parse a `model.toml` for editing.
///
/// The failure is the reader's, word for word: a file that is not TOML
/// must not be described one way when it is read and another when it is
/// edited. The reader's *second* failure -- deserializing into
/// `toml::Value`s -- is unreachable for anything that parses, since
/// duplicate keys are refused at parse time and every TOML type is a
/// `toml::Value`, so this matches only the parse.
/// Parse a `model.toml` for editing, after checking that it is one this
/// Sysand can read.
///
/// [`open`] is a bare parse: a file written for a `manifest-version`
/// this Sysand does not support parses perfectly well, and editing it
/// would be writing into a schema nothing here understands. So the same
/// read a project's [`crate::project::ProjectRead::manifest`] does runs
/// first, for its diagnostics alone, and the document is handed back
/// only when there are none.
pub fn validated(file: &str, text: &str) -> Result<DocumentMut, ProjectDeserializationError> {
    crate::manifest::ModelToml::parse(file, text)
        .map_err(|report| ProjectDeserializationError::invalid(file, report))?;
    open(file, text).map_err(|problem| {
        ProjectDeserializationError::invalid(
            file,
            crate::manifest::diagnostics::Report::from_iter([problem]),
        )
    })
}

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
/// Which of the two shapes holds them depends on what is written. A
/// scalar keeps the first on its own key and the second on its value; a
/// dotted marker (`version.workspace = true`) keeps *both* on the inner
/// `workspace` key and value, and its outer key carries nothing.
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
                let (above, after) = inner
                    .iter()
                    .next()
                    .map(|(name, _)| {
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
                    })
                    .unwrap_or_default();
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

    /// Put them back on a dotted marker's inner key and value.
    fn onto_marker(self, table: &mut Table, key: &str, mut marker: Table) {
        if let Some(after) = self.after
            && let Some(value) = marker.get_mut(MARKER).and_then(Item::as_value_mut)
        {
            value.decor_mut().set_suffix(after);
        }
        if let Some(above) = self.above
            && let Some(mut inner) = marker.key_mut(MARKER)
        {
            inner.leaf_decor_mut().set_prefix(above);
        }
        table.insert(key, Item::Table(marker));
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
/// A key that is not there is appended after the table's existing
/// values -- inside `[project]`, before the first sub-table header.
/// Not in the documented order of a generated file: putting a new key
/// in its documented place means moving the lines that are already
/// there, and every such move drags a comment to a place it no longer
/// describes.
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
/// `urls` is a table and goes through [`set_url_key`]; `root` belongs to
/// a schema v1 does not implement. Whether a value is one the project
/// accepts is the reader's question, as it is for every other key here.
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
/// model-toml.md allows three shapes and users write all of them: a
/// `[project.urls]` header, an inline `urls = { .. }`, and dotted
/// `urls.website = ..` keys. An existing table is edited in whichever
/// shape it has; a table that is not there is created as a *header*,
/// which is what a generated file has. Indexing into the document would
/// silently create the inline form instead.
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
/// The table is left even when the last key goes: removing it is a
/// second edit the caller did not ask for, and it would take whatever
/// comments are in it.
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

/// Replace a `[project]` key's value with an inheritance marker.
///
/// workspaces.md "Explicit inheritance" calls this one of the rewrites
/// that must preserve comments, and it is the one that moves them
/// furthest: a dotted marker keeps both its comments on its *inner*
/// key and value, where a scalar keeps them on the outer key and its
/// own value.
///
/// `urls` is the exception the rule cannot cover: a whole-table marker
/// replaces the table, so comments written inside `[project.urls]` are
/// gone with it. There is no line left for them to annotate.
pub fn set_inherit_marker(
    doc: &mut DocumentMut,
    key: &str,
    from: &InheritFrom,
) -> Result<(), EditError> {
    if !PROJECT_KEYS.contains(&key) {
        return Err(EditError::UnknownKey(key.to_owned()));
    }
    if !INHERITABLE_KEYS.contains(&key) {
        return Err(EditError::NotInheritable(key.to_owned()));
    }
    let project = project_mut(doc)?;
    let comments = KeyComments::read(project, key);
    comments.onto_marker(
        project,
        key,
        marker(&Inherited {
            from: from.clone(),
            at: None,
        }),
    );
    Ok(())
}

/// model-toml.md "Editing the manifest": "`sysand include <paths>`
/// appends each path to `sources`, normalized as on read, rejecting a
/// duplicate".
///
/// The whole array is re-validated, not just the new entry: `sources`
/// has rules that span entries -- a duplicate is one of them -- and
/// there is one implementation of them. An array that was already
/// invalid therefore fails naming an entry the caller did not touch,
/// which is one honest message rather than two readings of the rule.
pub fn include_source(doc: &mut DocumentMut, path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    let array = sources_mut(doc)?;
    let mut candidate = entries(array);
    candidate.push(Utf8UnixPathBuf::from(path));
    let normalized = Sources::new(candidate)?;
    let added = normalized
        .as_slice()
        .last()
        .expect("a path was just pushed")
        .clone();

    push_entry(array, added.as_str());
    Ok(added)
}

/// "`sysand exclude <paths>` removes it." Matching is by normalized
/// path, so `./a.sysml` removes an entry written as `a.sysml`.
pub fn exclude_source(doc: &mut DocumentMut, path: &str) -> Result<Utf8UnixPathBuf, EditError> {
    let wanted = Sources::new(vec![Utf8UnixPathBuf::from(path)])?
        .as_slice()
        .first()
        .expect("one path in, one path out")
        .clone();

    let array = sources_mut(doc)?;
    let at = entries(array)
        .iter()
        .position(|entry| {
            crate::utils::normalize_relative(entry.as_str())
                .ok()
                .as_ref()
                == Some(&wanted)
        })
        .ok_or_else(|| EditError::SourceNotListed(wanted.clone()))?;

    remove_entry(array, at);
    Ok(wanted)
}

/// Append one entry to a list-valued `[project]` key.
///
/// The array surgery of [`include_source`] is `sources`-only, and
/// `maintainers` and `topics` are arrays too. Reading such an array
/// out, changing it and writing it back with [`set_project_key`] would
/// replace the whole value and take every comment inside it -- which is
/// the one thing "Editing the manifest" forbids.
pub fn push_list_entry(doc: &mut DocumentMut, key: ListKey, value: &str) -> Result<(), EditError> {
    push_entry(list_mut(doc, key)?, value);
    Ok(())
}

/// Remove the entry at `at` from a list-valued `[project]` key.
///
/// The index is the caller's, and out of range is an error rather than
/// a silent no-op: `info topic --remove 7` on a list of three is a
/// mistake worth naming.
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
/// A key that is absent is created empty, so that `--add` on a project
/// that has never had one works; a key holding something that is not an
/// array is a manifest problem, not an edit.
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
/// `Array::push` gives the new entry no decor at all, so on a
/// multi-line array it lands on the previous entry's line. Three shapes
/// have to be told apart, and one of them repaired first.
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

    // An array written without a trailing comma keeps the newline
    // before `]` in the last entry's *suffix*, where a push would put
    // the comma after it -- `"A.kerml"\n,`. Move it where the rest of
    // this function expects to find it.
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
    // entry's line and stays there; the rest is the closing
    // indentation.
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
/// A TOML comment binds forward: a comment written at the end of one
/// entry's line is stored at the *front* of the next entry's prefix. So
/// the successor inherits the removed entry's prefix head -- everything
/// through its first newline, which is what was on the line above --
/// and keeps its own prefix tail. Exactly one thing goes: what was
/// physically on the removed entry's line.
///
/// The last entry has no successor and the array's trailing plays the
/// part, for the same reason: leaving it alone does not preserve a
/// comment, it re-binds it to the entry before.
fn remove_entry(array: &mut Array, at: usize) {
    let removed = prefix_of(array, at);
    let head = match removed.split_once('\n') {
        Some((head, _)) => format!("{head}\n"),
        // Nothing above: whatever is here was on the previous entry's
        // line, and is worth keeping only if it says something.
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
/// model-toml.md "Normalization" makes duplicates "the one comparison
/// that normalizes before resolution", so an entry naming the same
/// project under a different spelling is refused.
///
/// The entry lands after the last one and before whatever trails the
/// file: a comment at the end of a document belongs to no table, and
/// `toml_edit` keeps it where it is.
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
/// with it -- which is right, since it describes the dependency being
/// removed. It is the one comment this module deletes on purpose.
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
/// `Some(None)` is an entry with no `version` key -- a `dir`,
/// `kpar-path`, `url` or opt-in usage. `None` is no entry at all. A
/// caller that means to add a usage has to tell the two apart.
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
/// Only an index usage has one: a `dir`, `kpar-path` or `url` entry
/// names its target directly, and an entry that opts into a workspace
/// group takes that group's entry whole. Both are refused, and the
/// command that owns the verb decides how to say which it met.
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

    if ["dir", "kpar-path", "url", "git", MARKER]
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
/// A `members` entry is admitted only in its normalized form
/// (workspaces.md "Included projects"), so the path is written as
/// [`crate::utils::normalize_relative`] converts it: `./app/` is written
/// `app`, and is then the spelling `--member <path>` matches "literally"
/// (workspaces.md "Command scopes").
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
/// The identity is read off the document rather than through
/// `UsageRaw`, which reads a `toml::Table` and would mean serializing
/// each entry and parsing it back. An entry whose `publisher` or `name`
/// is absent, of the wrong type, or not a valid identifier yields
/// nothing: it is too broken to say which project it declares, so it is
/// neither matched nor in the way of the entry that is.
fn find_usage(doc: &DocumentMut, wanted: &NormalizedIdentity) -> Option<usize> {
    doc.get(USAGES)
        .and_then(Item::as_array_of_tables)?
        .iter()
        .position(|entry| entry_identity(entry).as_ref() == Some(wanted))
}

fn entry_identity(entry: &Table) -> Option<NormalizedIdentity> {
    let field = |key| entry.get(key).and_then(Item::as_str);
    let publisher = IndexPublisher::parse(field("publisher")?).ok()?;
    let name = IndexName::parse(field("name")?).ok()?;
    Some(normalized_identity(&publisher, &name))
}

#[cfg(test)]
#[path = "./edit_tests.rs"]
mod tests;
