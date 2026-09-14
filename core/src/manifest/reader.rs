// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Reading a `model.toml` table without letting one bad value stop the
//! read.
//!
//! model-toml.md "Reading and validating": "Parsing a manifest never
//! fails on its content: every field of every manifest ... is optional at
//! parse time. Whether a key is required, whether its value has the right
//! shape, and every rule that spans keys is checked by Sysand's own
//! validation, which reports every problem it finds in one report, with
//! the position of each."
//!
//! A `#[derive(Deserialize)]` struct cannot do that: a value of the wrong
//! TOML type fails inside `serde`, which aborts the whole file at the
//! first one. So a document is deserialized into plain [`toml::Value`]s,
//! which accept every type, and every judgement about a value is made
//! here, against the span index of [`crate::manifest::spans`].
//!
//! [`TableReader`] is what makes that bearable: it carries the four
//! things that are constant while one table is read -- the table, the
//! path that addresses it in the span index, the [`KeyLocation`] its
//! messages name, and the positions -- and turns "look the key up, check
//! its type, position the complaint" into one call.

use toml::Value;

use crate::manifest::common::Maintainer;
use crate::manifest::diagnostics::{Diagnostic, KeyLocation, Position, ReadWarning, Report};
use crate::manifest::spans::Positions;

/// How a message names the TOML type a value turned out to have, with
/// the article that reads in "but it is ...".
///
/// Public because every reader of a manifest value says the same thing
/// about a value of the wrong type, including the ones that do not go
/// through a [`TableReader`].
pub fn describe(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "a string",
        Value::Integer(_) => "an integer",
        Value::Float(_) => "a float",
        Value::Boolean(_) => "a boolean",
        Value::Datetime(_) => "a datetime",
        Value::Array(_) => "an array",
        Value::Table(_) => "a table",
    }
}

/// workspaces.md "Explicit inheritance": a marker is the table
/// `{ workspace = <bool-or-string> }` sitting where a plain value would
/// otherwise go; this is the value it carries, when it is one.
///
/// Recognized here so that a marker written where nothing can be
/// inherited says so, rather than coming back as a type error or, for a
/// table-valued key, as an unknown key inside it. Public because
/// [`crate::manifest::inherit`] recognizes the same shape where a marker
/// *is* allowed, and the two must agree on what one is.
pub fn marker_value(value: &Value) -> Option<&Value> {
    match value {
        Value::Table(table) if table.len() == 1 => table.get("workspace"),
        _ => None,
    }
}

fn is_marker(value: &Value) -> bool {
    marker_value(value).is_some()
}

/// One table of a `model.toml` being read.
#[derive(Clone, Debug)]
pub struct TableReader<'a> {
    table: &'a toml::Table,
    path: Vec<String>,
    location: KeyLocation,
    positions: &'a Positions<'a>,
}

impl<'a> TableReader<'a> {
    pub fn new(
        table: &'a toml::Table,
        path: Vec<String>,
        location: KeyLocation,
        positions: &'a Positions<'a>,
    ) -> Self {
        Self {
            table,
            path,
            location,
            positions,
        }
    }

    /// How messages about this table name it.
    pub fn location(&self) -> &KeyLocation {
        &self.location
    }

    /// The table as it was written, for the few readers that take it
    /// whole rather than key by key.
    pub fn raw(&self) -> &'a toml::Table {
        self.table
    }

    pub fn has(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }

    /// Where this key is, or -- for a key that is not written -- where
    /// the table that should carry it is.
    pub fn at(&self, key: &str) -> Option<Position> {
        self.positions.near(&self.segments(&[key]))
    }

    /// Where the table itself is.
    pub fn here(&self) -> Option<Position> {
        self.positions.near(&self.segments(&[]))
    }

    /// Where one element of an array-valued key is.
    pub fn at_element(&self, key: &str, index: usize) -> Option<Position> {
        self.positions
            .near(&self.segments(&[key, &index.to_string()]))
    }

    /// The positions this table was read against, for the readers that
    /// hand them on (a usage entry carries its own site).
    pub fn positions(&self) -> &'a Positions<'a> {
        self.positions
    }

    /// One warning per key the caller did not ask for (workspaces.md
    /// "Unknown keys"), each at its own key. `metamodel` is named in
    /// the "this may be because" suffix: an unrecognized metamodel may
    /// be why the key is not known.
    pub fn unknown_keys(&self, known: &[&str], metamodel: Option<&str>) -> Vec<Diagnostic> {
        self.table
            .keys()
            .filter(|key| !known.contains(&key.as_str()))
            .map(|key| {
                Diagnostic::from(ReadWarning::UnknownKey {
                    location: self.location.clone(),
                    key: key.clone(),
                    unrecognized_metamodel: metamodel.map(ToOwned::to_owned),
                })
                .at_opt(self.at(key))
            })
            .collect()
    }

    /// The value of a key that holds a string.
    pub fn string(&self, key: &str, report: &mut Report) -> Option<&'a str> {
        let value = self.table.get(key)?;
        if let Value::String(text) = value {
            return Some(text.as_str());
        }
        report.push(self.wrong_type(key, "a string", value));
        None
    }

    /// The values of a key that holds an array of strings. Every element
    /// that is not one is reported on its own, so one bad entry does not
    /// cost the reader the rest of the array.
    pub fn strings(&self, key: &str, report: &mut Report) -> Option<Vec<&'a str>> {
        let value = self.table.get(key)?;
        let Value::Array(elements) = value else {
            report.push(self.wrong_type(key, "an array of strings", value));
            return None;
        };
        let mut strings = Vec::with_capacity(elements.len());
        let mut complete = true;
        for (index, element) in elements.iter().enumerate() {
            if let Value::String(text) = element {
                strings.push(text.as_str());
            } else {
                report.push(self.wrong_element(key, index, "a string", element));
                complete = false;
            }
        }
        complete.then_some(strings)
    }

    /// A reader for a key that holds a table.
    pub fn table(&self, key: &str, report: &mut Report) -> Option<Self> {
        let value = self.table.get(key)?;
        // A marker is a table too, so it has to be refused before the
        // shape is accepted.
        if let Value::Table(table) = value
            && !is_marker(value)
        {
            return Some(Self::new(
                table,
                self.segments_owned(&[key]),
                nested(&self.location, key),
                self.positions,
            ));
        }
        report.push(self.wrong_type(key, "a table", value));
        None
    }

    /// A reader per entry of a key that holds an array of tables
    /// (`[[inline-usages]]`). An absent key is an empty array, since
    /// nothing distinguishes the two.
    pub fn tables(&self, key: &str, report: &mut Report) -> Vec<Self> {
        let Some(value) = self.table.get(key) else {
            return Vec::new();
        };
        let Value::Array(elements) = value else {
            report.push(self.wrong_type(key, "an array of tables", value));
            return Vec::new();
        };
        let location = KeyLocation::ArrayTable(dotted(&self.location, key));
        let mut entries = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            if let Value::Table(table) = element {
                entries.push(Self::new(
                    table,
                    self.segments_owned(&[key, &index.to_string()]),
                    location.clone(),
                    self.positions,
                ));
            } else {
                report.push(self.wrong_element(key, index, "a table", element));
            }
        }
        entries
    }

    /// A reader per entry of a key that holds a *table of tables* keyed
    /// by name (`[workspace.group.<name>]`), in the order the file writes
    /// them.
    pub fn named_tables(&self, key: &str, report: &mut Report) -> Vec<(&'a str, Self)> {
        let Some(outer) = self.table(key, report) else {
            return Vec::new();
        };
        let mut entries = Vec::with_capacity(outer.table.len());
        for (name, value) in outer.table {
            if let Value::Table(table) = value {
                entries.push((
                    name.as_str(),
                    Self::new(
                        table,
                        outer.segments_owned(&[name]),
                        nested(&outer.location, name),
                        self.positions,
                    ),
                ));
            } else {
                report.push(outer.wrong_type(name, "a table", value));
            }
        }
        entries
    }

    fn segments<'b>(&'b self, tail: &[&'b str]) -> Vec<&'b str> {
        self.path
            .iter()
            .map(String::as_str)
            .chain(tail.iter().copied())
            .collect()
    }

    fn segments_owned(&self, tail: &[&str]) -> Vec<String> {
        self.segments(tail)
            .into_iter()
            .map(ToOwned::to_owned)
            .collect()
    }

    fn wrong_type(&self, key: &str, expected: &str, value: &Value) -> Diagnostic {
        if is_marker(value) {
            return Diagnostic::error(format!(
                "`{key}` in {} is an inheritance marker, but a workspace has nothing to \
                 inherit from; give the value directly",
                self.location
            ))
            .at_opt(self.at(key));
        }
        Diagnostic::error(format!(
            "`{key}` in {} must be {expected}, but it is {}",
            self.location,
            describe(value)
        ))
        .at_opt(self.at(key))
    }

    fn wrong_element(&self, key: &str, index: usize, expected: &str, value: &Value) -> Diagnostic {
        Diagnostic::error(format!(
            "entry {index} of `{key}` in {} must be {expected}, but it is {}",
            self.location,
            describe(value)
        ))
        .at_opt(
            self.positions
                .near(&self.segments(&[key, &index.to_string()])),
        )
    }
}

/// The dotted name of a key of this table, as a `[table]` header spells
/// it.
fn dotted(location: &KeyLocation, key: &str) -> String {
    match location {
        KeyLocation::Table(table) | KeyLocation::ArrayTable(table) => format!("{table}.{key}"),
        KeyLocation::TopLevel | KeyLocation::File(_) => key.to_owned(),
    }
}

fn nested(location: &KeyLocation, key: &str) -> KeyLocation {
    KeyLocation::Table(dotted(location, key))
}

/// A value whose own error already says everything (an identifier, a
/// maintainer): the message is the error's, positioned at the key.
pub fn parsed<T, E: std::fmt::Display>(
    result: Result<T, E>,
    reader: &TableReader<'_>,
    key: &str,
    report: &mut Report,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            report.push(Diagnostic::error(error.to_string()).at_opt(reader.at(key)));
            None
        }
    }
}

/// A value whose error does not name the key or the value it came from,
/// so the message says both and then does not hide the underlying error.
pub fn checked<T, E: std::fmt::Display>(
    result: Result<T, E>,
    reader: &TableReader<'_>,
    key: &str,
    raw: &str,
    expected: &str,
    report: &mut Report,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            report.push(
                Diagnostic::error(format!(
                    "{} sets `{key} = \"{raw}\"`, which is not {expected}: {error}",
                    reader.location()
                ))
                .at_opt(reader.at(key)),
            );
            None
        }
    }
}

/// model-toml.md `maintainers`: "array of strings, each of the form
/// `name <email>`". Every entry that is not is reported at itself.
pub fn maintainers(reader: &TableReader<'_>, report: &mut Report) -> Option<Vec<Maintainer>> {
    let raw = reader.strings("maintainers", report)?;
    let mut maintainers = Vec::with_capacity(raw.len());
    let mut complete = true;
    for (index, entry) in raw.iter().enumerate() {
        match Maintainer::parse(entry) {
            Ok(maintainer) => maintainers.push(maintainer),
            Err(error) => {
                report.push(
                    Diagnostic::error(error.to_string())
                        .at_opt(reader.at_element("maintainers", index)),
                );
                complete = false;
            }
        }
    }
    complete.then_some(maintainers)
}

#[cfg(test)]
#[path = "./reader_tests.rs"]
mod tests;
