// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Reading a `model.toml` table without letting one bad value stop the
//! read.
//!
//! Parsing a manifest never fails on its content: every field is
//! optional at parse time. Whether a key is required, whether its value
//! has the right shape, and every rule that spans keys is checked by
//! Sysand's own validation, which reports every problem it finds in one
//! report, with the position of each.
//!
//! Values are read straight off the parsed [`DeTable`], which accepts every
//! type and keeps the span of every key and value; every judgement about a
//! value is made here.
//!
//! [`TableReader`] carries the four things that are constant while one
//! table is read (the table, where the table itself is written, the
//! [`KeyLocation`] its messages name, and the [`Source`] that turns a span
//! into a position) and turns "look the key up, check its type, position
//! the complaint" into one call.

use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::manifest::common::Maintainer;
use crate::manifest::diagnostics::{Diagnostic, Diagnostics, KeyLocation, Position, ReadWarning};
use crate::manifest::spans::Source;
use crate::model::KnownMetamodel;

/// How a message names a TOML type, given as the `type_str` of a value,
/// with the article that reads in "but it is ...".
pub fn describe(type_str: &str) -> &'static str {
    match type_str {
        "string" => "a string",
        "integer" => "an integer",
        "float" => "a float",
        "boolean" => "a boolean",
        "datetime" => "a datetime",
        "array" => "an array",
        "table" => "a table",
        _ => "a value of an unknown type",
    }
}

/// A marker is the table `{ workspace = <bool-or-string> }` sitting where
/// a plain value would otherwise go; this is the value it carries, when it
/// is one.
pub fn marker_value<'v, 'i>(value: &'v DeValue<'i>) -> Option<&'v DeValue<'i>> {
    match value {
        DeValue::Table(table) if table.len() == 1 => table.get("workspace").map(Spanned::get_ref),
        _ => None,
    }
}

fn is_marker(value: &DeValue<'_>) -> bool {
    marker_value(value).is_some()
}

/// One table of a `model.toml` being read.
#[derive(Clone, Debug)]
pub struct TableReader<'a> {
    table: &'a DeTable<'a>,
    /// Where the table itself is written: the key that names it, or the
    /// `[[...]]` header of an entry of an array of tables. `None` for the
    /// document itself and for a table the file does not write.
    span: Option<Range<usize>>,
    location: KeyLocation,
    source: &'a Source<'a>,
}

impl<'a> TableReader<'a> {
    pub fn new(
        table: &'a DeTable<'a>,
        span: Option<Range<usize>>,
        location: KeyLocation,
        source: &'a Source<'a>,
    ) -> Self {
        Self {
            table,
            span,
            location,
            source,
        }
    }

    /// How messages about this table name it.
    pub fn location(&self) -> &KeyLocation {
        &self.location
    }

    /// The table as it was written, for the few readers that take it
    /// whole.
    pub fn raw(&self) -> &'a DeTable<'a> {
        self.table
    }

    /// The value of a key, whatever its type.
    pub fn get(&self, key: &str) -> Option<&'a Spanned<DeValue<'a>>> {
        self.table.get(key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.table.contains_key(key)
    }

    /// The file this table was read from, for the readers that position
    /// what they find themselves.
    pub fn source(&self) -> &'a Source<'a> {
        self.source
    }

    /// Where this key is, or for a key that is not written, where the
    /// table that should carry it is.
    pub fn at(&self, key: &str) -> Option<Position> {
        match self.table.get_key_value(key) {
            Some((key, _)) => Some(self.source.position(key.span())),
            None => self.here(),
        }
    }

    /// Where the table itself is.
    pub fn here(&self) -> Option<Position> {
        self.span.clone().map(|span| self.source.position(span))
    }

    /// Where one element of an array-valued key is; for an element that is
    /// not there, where the key is.
    pub fn at_element(&self, key: &str, index: usize) -> Option<Position> {
        match self.get(key).map(Spanned::get_ref) {
            Some(DeValue::Array(elements)) if index < elements.len() => {
                Some(self.source.position(elements[index].span()))
            }
            _ => self.at(key),
        }
    }

    /// One warning per key not in `known`, each at its own key.
    /// `metamodel` is named in the "this may be because" suffix.
    pub fn unknown_keys(&self, known: &[&str], metamodel: Option<&str>) -> Vec<Diagnostic> {
        self.table
            .keys()
            .filter(|key| !known.contains(&key.get_ref().as_ref()))
            .map(|key| {
                Diagnostic::from(ReadWarning::UnknownKey {
                    location: self.location.clone(),
                    key: key.get_ref().clone().into_owned(),
                    unrecognized_metamodel: metamodel.map(ToOwned::to_owned),
                })
                .at(self.source.position(key.span()))
            })
            .collect()
    }

    /// The value of a key that holds a string.
    pub fn string(&self, key: &str, report: &mut Diagnostics) -> Option<&'a str> {
        let value = self.get(key)?.get_ref();
        if let DeValue::String(text) = value {
            return Some(text);
        }
        report.push(self.wrong_type(key, "a string", value));
        None
    }

    /// The values of a key that holds an array of strings. Every element
    /// that is not one is reported on its own.
    pub fn strings(&self, key: &str, report: &mut Diagnostics) -> Option<Vec<&'a str>> {
        let value = self.get(key)?.get_ref();
        let DeValue::Array(elements) = value else {
            report.push(self.wrong_type(key, "an array of strings", value));
            return None;
        };
        let mut strings = Vec::with_capacity(elements.len());
        let mut complete = true;
        for (index, element) in elements.iter().enumerate() {
            if let DeValue::String(text) = element.get_ref() {
                strings.push(&**text);
            } else {
                report.push(self.wrong_element(key, index, "a string", element));
                complete = false;
            }
        }
        complete.then_some(strings)
    }

    /// A reader for a key that holds a table.
    pub fn table(&self, key: &str, report: &mut Diagnostics) -> Option<Self> {
        let (name, value) = self.table.get_key_value(key)?;
        let value = value.get_ref();
        // A marker is a table too, but not one to read.
        if let DeValue::Table(table) = value
            && !is_marker(value)
        {
            return Some(Self::new(
                table,
                Some(name.span()),
                nested(&self.location, key),
                self.source,
            ));
        }
        report.push(self.wrong_type(key, "a table", value));
        None
    }

    /// A reader per entry of a key that holds an array of tables
    /// (`[[inline-usages]]`). An absent key is an empty array, since
    /// nothing distinguishes the two.
    pub fn tables(&self, key: &str, report: &mut Diagnostics) -> Vec<Self> {
        let Some(value) = self.get(key) else {
            return Vec::new();
        };
        let DeValue::Array(elements) = value.get_ref() else {
            report.push(self.wrong_type(key, "an array of tables", value.get_ref()));
            return Vec::new();
        };
        let location = KeyLocation::ArrayTable(dotted(&self.location, key));
        let mut entries = Vec::with_capacity(elements.len());
        for (index, element) in elements.iter().enumerate() {
            // An entry is where its `[[...]]` header is, or, written
            // inline, where its `{ ... }` is.
            if let DeValue::Table(table) = element.get_ref() {
                entries.push(Self::new(
                    table,
                    Some(element.span()),
                    location.clone(),
                    self.source,
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
    pub fn named_tables(&self, key: &str, report: &mut Diagnostics) -> Vec<(&'a str, Self)> {
        let Some(outer) = self.table(key, report) else {
            return Vec::new();
        };
        let mut entries = Vec::with_capacity(outer.table.len());
        for (name, value) in outer.table {
            let name_text: &'a str = name.get_ref();
            if let DeValue::Table(table) = value.get_ref() {
                entries.push((
                    name_text,
                    Self::new(
                        table,
                        Some(name.span()),
                        nested(&outer.location, name_text),
                        self.source,
                    ),
                ));
            } else {
                report.push(outer.wrong_type(name_text, "a table", value.get_ref()));
            }
        }
        entries
    }

    fn wrong_type(&self, key: &str, expected: &str, value: &DeValue<'_>) -> Diagnostic {
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
            describe(value.type_str())
        ))
        .at_opt(self.at(key))
    }

    fn wrong_element(
        &self,
        key: &str,
        index: usize,
        expected: &str,
        element: &Spanned<DeValue<'_>>,
    ) -> Diagnostic {
        Diagnostic::error(format!(
            "entry {index} of `{key}` in {} must be {expected}, but it is {}",
            self.location,
            describe(element.get_ref().type_str())
        ))
        .at(self.source.position(element.span()))
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
    report: &mut Diagnostics,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            report.push(Diagnostic::error(error.to_string()).at_opt(reader.at(key)));
            None
        }
    }
}

/// Read the `metamodel` key. Exactly four values are recognized; any
/// other is an error naming the key and listing them.
pub fn metamodel(reader: &TableReader<'_>, report: &mut Diagnostics) -> Option<KnownMetamodel> {
    let raw = reader.string("metamodel", report)?;
    let metamodel = KnownMetamodel::recognize(raw);
    if metamodel.is_none() {
        report.push(
            Diagnostic::error(format!(
                "`metamodel` in {} is `{raw}`, which is not one of the four values a \
                 `model.toml` recognizes;\nthey are {list}",
                reader.location(),
                list = KnownMetamodel::RECOGNIZED,
            ))
            .at_opt(reader.at("metamodel")),
        );
    }
    metamodel
}

/// A value whose error omits the key and the value it came from; the
/// message names both and keeps the underlying error.
pub fn checked<T, E: std::fmt::Display>(
    result: Result<T, E>,
    reader: &TableReader<'_>,
    key: &str,
    raw: &str,
    expected: &str,
    report: &mut Diagnostics,
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

/// An array of strings, each of the form `name <email>`. Every entry that
/// is not is reported at itself.
pub fn maintainers(reader: &TableReader<'_>, report: &mut Diagnostics) -> Option<Vec<Maintainer>> {
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
