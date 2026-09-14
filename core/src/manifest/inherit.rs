// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Explicit inheritance: a `[project]` key that takes its value from a
//! workspace group instead of giving one.
//!
//! workspaces.md "Explicit inheritance": "In every project, to inherit a
//! key from the default group, `<key>.workspace = true` is used. To
//! inherit from a non-default group, `<key>.workspace = \"group-name\"`."
//! The marker is written where the value would be, so reading such a key
//! means deciding, from the shape of what is written, which of the two it
//! is.
//!
//! This module knows nothing about which keys `[project]` has. It knows
//! what a marker looks like, what is wrong with the shapes that are not
//! markers, and how to follow a marker into a
//! [`GroupProperties`](crate::manifest::workspace_table::GroupProperties);
//! `project.rs` knows the key list.

use crate::manifest::common::{InheritFrom, InheritMarker};
use crate::manifest::diagnostics::{Diagnostic, Position, Report};
use crate::manifest::reader::{TableReader, describe, marker_value};
use crate::manifest::workspace_table::{GroupProperties, WorkspaceTable};

/// A marker that was read: which group it points at, and where it was
/// written.
///
/// The position is kept because the diagnostics a marker can produce are
/// raised by `resolve`, which runs once the workspace is known and has no
/// text to look positions up in: a `Positions<'a>` borrows one file's
/// text, and a workspace resolves many members into one report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inherited {
    pub from: InheritFrom,
    pub at: Option<Position>,
}

/// A value a `[project]` key may either give directly or inherit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaProperty<T> {
    Provided(T),
    Inherited(Inherited),
}

impl<T> MetaProperty<T> {
    /// The value, for a key that gives one directly.
    pub fn provided(&self) -> Option<&T> {
        match self {
            Self::Provided(value) => Some(value),
            Self::Inherited(_) => None,
        }
    }

    /// The marker, for a key that carries one.
    pub fn inherited(&self) -> Option<&Inherited> {
        match self {
            Self::Inherited(inherited) => Some(inherited),
            Self::Provided(_) => None,
        }
    }
}

/// Read a key that may carry an inheritance marker and whose own value is
/// a scalar or an array.
///
/// `read` is what reads the value when there is no marker -- an ordinary
/// [`TableReader`] call -- and is not run at all when there is one.
pub fn inheritable<T>(
    reader: &TableReader<'_>,
    key: &str,
    report: &mut Report,
    read: impl FnOnce(&mut Report) -> Option<T>,
) -> Option<MetaProperty<T>> {
    match reader.raw().get(key).and_then(marker_value) {
        Some(marker) => read_marker(reader, key, marker, report).map(MetaProperty::Inherited),
        // A `workspace` key among others is not a marker. For a scalar
        // key it is not an overlap either (see `inheritable_table`): the
        // value is simply a table where a table cannot go, which is what
        // `read` says.
        None => read(report).map(MetaProperty::Provided),
    }
}

/// The same, for a key whose own value is a *table* (`urls`).
///
/// workspaces.md: "when a key is inherited from a table and the key's
/// value is also a table, inheritance cannot be specified for any of the
/// inner table's keys, nor can the inner key be given a value of its
/// own". Such a table has a `workspace` key and others, so it is not a
/// marker -- and without this check its `workspace` key would come back
/// as an unknown key inside the table, a warning that leaves the read to
/// succeed with the inheritance silently dropped.
pub fn inheritable_table<T>(
    reader: &TableReader<'_>,
    key: &str,
    report: &mut Report,
    read: impl FnOnce(&mut Report) -> Option<T>,
) -> Option<MetaProperty<T>> {
    if let Some(toml::Value::Table(table)) = reader.raw().get(key)
        && table.len() > 1
        && table.contains_key("workspace")
    {
        let others = table
            .keys()
            .filter(|name| name.as_str() != "workspace")
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        report.push(
            Diagnostic::error(format!(
                "inheritance cannot overlap: `{key}` in {} carries a `workspace` marker and \
                 also sets {others};\n\
                 a key is either inherited or written out in full",
                reader.location()
            ))
            .at_opt(reader.at(key)),
        );
        return None;
    }
    inheritable(reader, key, report, read)
}

/// Read a key that may *not* be inherited.
///
/// workspaces.md "Common metadata" lists the `[project]` keys that are
/// deliberately outside inheritance. A marker on one of them says so,
/// rather than reporting a table where an array or a string was expected
/// -- and, unlike an inheritable key, `workspace = false` gets the same
/// answer, since what is wrong is the attempt and not the marker's value.
///
/// `reason` completes "cannot be inherited from the workspace; ...".
pub fn own<T>(
    reader: &TableReader<'_>,
    key: &str,
    reason: &str,
    report: &mut Report,
    read: impl FnOnce(&mut Report) -> Option<T>,
) -> Option<T> {
    if reader.raw().get(key).and_then(marker_value).is_some() {
        report.push(
            Diagnostic::error(format!(
                "`{key}` in {} cannot be inherited from the workspace;\n{reason}",
                reader.location()
            ))
            .at_opt(reader.at(key)),
        );
        return None;
    }
    read(report)
}

fn read_marker(
    reader: &TableReader<'_>,
    key: &str,
    marker: &toml::Value,
    report: &mut Report,
) -> Option<Inherited> {
    let at = reader.at(key);
    let marker = match marker {
        toml::Value::Boolean(flag) => InheritMarker::Flag(*flag),
        toml::Value::String(group) => InheritMarker::Group(group.clone()),
        other => {
            report.push(
                Diagnostic::error(format!(
                    "`{key}.workspace` in {} must be a boolean or a string, but it is {}",
                    reader.location(),
                    describe(other)
                ))
                .at_opt(at),
            );
            return None;
        }
    };
    match InheritFrom::try_from(marker) {
        Ok(from) => Some(Inherited { from, at }),
        Err(error) => {
            report.push(Diagnostic::error(error.to_string()).at_opt(at));
            None
        }
    }
}

/// Follow a marker into the group it names, and take one key out of it.
///
/// workspaces.md: "A marker must find a value: naming a group
/// that does not exist, or one that does not define the key, is an error,
/// whether or not the key is required in `[project]`."
pub fn from_group<U>(
    inherited: &Inherited,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<U>,
) -> Result<U, Diagnostic> {
    let from = &inherited.from;
    let at = inherited.at.clone();
    // workspaces.md: "Markers resolve only against the workspace the
    // project is a *member* of. A project that is not a member is outside
    // every workspace regardless of where its directory sits."
    let Some(workspace) = workspace else {
        return Err(Diagnostic::error(format!(
            "`{key}` inherits from {from}, but the project is not a member of any workspace"
        ))
        .at_opt(at));
    };
    // Only a named group can be absent: the default group is the keys of
    // `[workspace]` itself, which always exist.
    let Some(group) = workspace.group_properties(from) else {
        return Err(Diagnostic::error(format!(
            "`{key}` inherits from {from}, which the workspace does not define"
        ))
        .at_opt(at));
    };
    pick(group).ok_or_else(|| {
        Diagnostic::error(format!(
            "`{key}` inherits from {from}, which does not give it"
        ))
        .at_opt(at)
    })
}

/// Resolve one key that a `[project]` must have: either the value it
/// gives, or the one its marker points at.
pub fn resolve_property<T, U>(
    property: MetaProperty<T>,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<U>,
    convert: impl FnOnce(T) -> U,
) -> Result<U, Diagnostic> {
    match property {
        MetaProperty::Provided(value) => Ok(convert(value)),
        MetaProperty::Inherited(inherited) => from_group(&inherited, key, workspace, pick),
    }
}

/// The same for a key that may be absent, which is not the same as a
/// group that does not give it.
pub fn resolve_optional<T, U>(
    property: Option<MetaProperty<T>>,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<U>,
    convert: impl FnOnce(T) -> U,
) -> Result<Option<U>, Diagnostic> {
    property
        .map(|property| resolve_property(property, key, workspace, pick, convert))
        .transpose()
}

#[cfg(test)]
#[path = "./inherit_tests.rs"]
mod tests;
