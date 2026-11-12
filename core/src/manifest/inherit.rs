// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Explicit inheritance: a `[project]` key that takes its value from a
//! workspace group.
//!
//! To inherit a key from the default group, `<key>.workspace = true` is
//! used; to inherit from a non-default group,
//! `<key>.workspace = "group-name"`. The marker is written where the
//! value would be, so reading such a key means deciding, from the shape
//! of what is written, which of the two it is.
//!
//! This module knows nothing about which keys `[project]` has. It knows
//! what a marker looks like, what is wrong with the shapes that are not
//! markers, and how to follow a marker into a
//! [`GroupProperties`]. The key list belongs to the `[project]` reader.

use toml::Spanned;
use toml::de::DeValue;

use crate::manifest::common::{InheritFrom, InheritMarker};
use crate::manifest::diagnostics::{Diagnostic, Diagnostics, Position};
use crate::manifest::reader::{TableReader, describe};
use crate::manifest::workspace_table::{GroupProperties, WorkspaceTable};

/// A marker that was read: which group it points at, and where it was
/// written.
///
/// The position is kept for the diagnostics raised by `resolve`, which has
/// no file text to look positions up in.
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
/// `read` reads the value when there is no marker and does not run when
/// there is one.
pub fn inheritable<T>(
    reader: &TableReader<'_>,
    key: &str,
    report: &mut Diagnostics,
    read: impl FnOnce(&mut Diagnostics) -> Option<T>,
) -> Option<MetaProperty<T>> {
    match reader.marker(key) {
        Some(marker) => read_marker(reader, key, marker, report).map(MetaProperty::Inherited),
        // Not a marker (including a `workspace` key among others): `read`
        // reports a table where none can go.
        None => read(report).map(MetaProperty::Provided),
    }
}

/// The same, for a key whose own value is a *table* (`urls`).
///
/// A table with a `workspace` key and others is not a marker: inheritance
/// cannot be specified for any of the inner table's keys, nor can the inner
/// key be given a value of its own. Without this check its `workspace` key
/// would come back as an unknown-key warning with the inheritance silently
/// dropped.
pub fn inheritable_table<T>(
    reader: &TableReader<'_>,
    key: &str,
    report: &mut Diagnostics,
    read: impl FnOnce(&mut Diagnostics) -> Option<T>,
) -> Option<MetaProperty<T>> {
    if let Some(DeValue::Table(table)) = reader.get(key).map(Spanned::get_ref)
        && table.len() > 1
        && table.contains_key("workspace")
    {
        let others = table
            .keys()
            .map(Spanned::get_ref)
            .filter(|name| *name != "workspace")
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
/// Some `[project]` keys are outside inheritance. A marker on one of them,
/// `workspace = false` included, is an error.
///
/// `reason` completes "cannot be inherited from the workspace; ...".
pub fn own<T>(
    reader: &TableReader<'_>,
    key: &str,
    reason: &str,
    report: &mut Diagnostics,
    read: impl FnOnce(&mut Diagnostics) -> Option<T>,
) -> Option<T> {
    if reader.marker(key).is_some() {
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
    marker: &DeValue<'_>,
    report: &mut Diagnostics,
) -> Option<Inherited> {
    let at = reader.at(key);
    let marker = match marker {
        DeValue::Boolean(flag) => InheritMarker::Flag(*flag),
        DeValue::String(group) => InheritMarker::Group(group.clone().into_owned()),
        other => {
            report.push(
                Diagnostic::error(format!(
                    "`{key}.workspace` in {} must be a boolean or a string, but it is {}",
                    reader.location(),
                    describe(other.type_str())
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
/// A marker must find a value: naming a group that does not exist, or
/// one that does not define the key, is an error, whether or not the key
/// is required in `[project]`.
pub fn from_group<U>(
    inherited: &Inherited,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<U>,
) -> Result<U, Diagnostic> {
    let from = &inherited.from;
    let at = inherited.at.clone();
    let group = group_named(&format!("`{key}`"), from, at.clone(), workspace)?;
    pick(group).ok_or_else(|| {
        Diagnostic::error(format!(
            "`{key}` inherits from {from}, which does not give it"
        ))
        .at_opt(at)
    })
}

/// The group a marker names. `subject` names what inherits in the
/// errors.
pub fn group_named<'a>(
    subject: &str,
    from: &InheritFrom,
    at: Option<Position>,
    workspace: Option<&'a WorkspaceTable>,
) -> Result<&'a GroupProperties, Diagnostic> {
    // Markers resolve only against the workspace the project is a member of.
    let Some(workspace) = workspace else {
        return Err(Diagnostic::error(format!(
            "{subject} inherits from {from}, but the project is not a member of any workspace"
        ))
        .at_opt(at));
    };
    // Only a named group can be absent: the default group is the keys of
    // `[workspace]` itself, which always exist.
    workspace.group_properties(from).ok_or_else(|| {
        Diagnostic::error(format!(
            "{subject} inherits from {from}, which the workspace does not define"
        ))
        .at_opt(at)
    })
}

/// Resolve one key that a `[project]` must have: either the value it
/// gives, or the one its marker points at.
pub fn resolve_property<T>(
    property: MetaProperty<T>,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<T>,
) -> Result<T, Diagnostic> {
    match property {
        MetaProperty::Provided(value) => Ok(value),
        MetaProperty::Inherited(inherited) => from_group(&inherited, key, workspace, pick),
    }
}

/// The same for a key that may be absent, which is not the same as a
/// group that does not give it.
pub fn resolve_optional<T>(
    property: Option<MetaProperty<T>>,
    key: &str,
    workspace: Option<&WorkspaceTable>,
    pick: impl FnOnce(&GroupProperties) -> Option<T>,
) -> Result<Option<T>, Diagnostic> {
    property
        .map(|property| resolve_property(property, key, workspace, pick))
        .transpose()
}

#[cfg(test)]
#[path = "./inherit_tests.rs"]
mod tests;
