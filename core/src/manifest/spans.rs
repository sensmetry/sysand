// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Where every key of a `model.toml` is, so that a diagnostic can point
//! at the one that caused it.
//!
//! Positions come from `Key::span` of a parsed `toml_edit::Document`.
//! The document is consumed by the deserializer
//! (`toml_edit::de::Deserializer::from`), which is why the spans are
//! harvested into an owned index first:
//!
//! ```ignore
//! let document = toml_edit::Document::parse(text)?;
//! let spans = SpanIndex::harvest(&document);
//! let table = toml::Table::deserialize(toml_edit::de::Deserializer::from(document))?;
//! ```
//!
//! [`Source::read`] does that, and is the only way to get [`Positions`]
//! from text.
//!
//! The text is parsed once: the deserializer keeps it, so a failure
//! inside serde still reports a span of its own.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ops::Range;

use serde::Deserialize as _;
use toml_edit::{Document, DocumentMut, Item, Table, Value};

use crate::manifest::diagnostics::{Diagnostic, Position};

/// The span of every key of a parsed document, addressed by the path of
/// key segments that leads to it.
///
/// Segments rather than one dotted string, because TOML lets a quoted key
/// contain a dot: `"a.b" = 1` and `[a]` with `b = 2` are different keys
/// that may both appear in one file, and both would flatten to `a.b`.
///
/// An element of an array is addressed by its index
/// (`["inline-usages", "0"]`, `["members", "1"]`). An element of an array
/// of *tables* holds the span of its `[[...]]` header, which is what a
/// diagnostic about the entry as a whole points at; every other element
/// holds the span of its own value, since it has no key of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanIndex(BTreeMap<Vec<String>, Range<usize>>);

impl SpanIndex {
    /// Walk a parsed document, recording where each key is.
    pub fn harvest<S: AsRef<str>>(document: &Document<S>) -> Self {
        let mut index = Self::default();
        let mut path = Vec::new();
        index.walk_table(&mut path, document.as_table());
        index
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The span recorded for exactly this path, if there is one.
    pub fn get(&self, path: &[&str]) -> Option<&Range<usize>> {
        let path: Vec<String> = path.iter().map(|segment| (*segment).to_owned()).collect();
        self.0.get(&path)
    }

    /// The span of this path, or of the longest prefix of it that has
    /// one. A key that is missing has no span of its own, so a diagnostic
    /// about it points at the table or entry it is missing from.
    pub fn get_near(&self, path: &[&str]) -> Option<&Range<usize>> {
        (0..=path.len())
            .rev()
            .find_map(|length| self.get(&path[..length]))
    }

    /// First writes win: a path that is somehow reached twice keeps the
    /// span found first, so an unforeseen collision costs precision
    /// rather than correctness.
    fn insert(&mut self, path: &[String], span: Option<Range<usize>>) {
        let Some(span) = span else {
            return;
        };
        if let Entry::Vacant(entry) = self.0.entry(path.to_vec()) {
            entry.insert(span);
        }
    }

    fn walk_table(&mut self, path: &mut Vec<String>, table: &Table) {
        for (key, item) in table {
            let Some((key, _)) = table.get_key_value(key) else {
                continue;
            };
            path.push(key.get().to_owned());
            self.insert(path, key.span());
            self.walk_item(path, item);
            path.pop();
        }
    }

    fn walk_item(&mut self, path: &mut Vec<String>, item: &Item) {
        match item {
            Item::Table(table) => self.walk_table(path, table),
            Item::ArrayOfTables(tables) => {
                for (index, table) in tables.iter().enumerate() {
                    path.push(index.to_string());
                    // The `[[...]]` header, not the entry's body.
                    self.insert(path, table.span());
                    self.walk_table(path, table);
                    path.pop();
                }
            }
            Item::Value(value) => self.walk_value(path, value),
            Item::None => {}
        }
    }

    fn walk_value(&mut self, path: &mut Vec<String>, value: &Value) {
        match value {
            // An inline table is the same table written differently, so
            // it is indexed the same way.
            Value::InlineTable(table) => {
                for (key, value) in table {
                    let Some((key, _)) = table.get_key_value(key) else {
                        continue;
                    };
                    path.push(key.get().to_owned());
                    self.insert(path, key.span());
                    self.walk_value(path, value);
                    path.pop();
                }
            }
            // An element of an array is addressed by its index, exactly
            // as an element of an array of tables is. It has no key of
            // its own, so what is recorded is the span of the value --
            // the one place in this index where that is so.
            Value::Array(array) => {
                for (index, element) in array.iter().enumerate() {
                    path.push(index.to_string());
                    self.insert(path, element.span());
                    self.walk_value(path, element);
                    path.pop();
                }
            }
            // Every other value is a leaf, and the key that holds it is
            // already indexed.
            _ => {}
        }
    }
}

/// A file read as TOML, under the name its messages call it by.
///
/// The only thing that turns a byte span into a [`Position`]: every span
/// it turns is one that parsing its own text produced -- the spans of a
/// [`Positions`] it built, or the span of a parse error -- so a span always
/// falls on a character of the text it is measured against.
#[derive(Clone, Debug)]
pub struct Source<'a> {
    file: String,
    text: &'a str,
}

impl<'a> Source<'a> {
    pub fn new(file: impl Into<String>, text: &'a str) -> Self {
        Self {
            file: file.into(),
            text,
        }
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    /// The values a reader judges and the positions a diagnostic points
    /// at.
    ///
    /// The text is parsed once: [`SpanIndex::harvest`] takes what it
    /// needs from the immutable document before the deserializer consumes
    /// it.
    ///
    /// The `Err` is the one failure that stops a read before any rule of
    /// the manifest runs -- the file is not TOML at all.
    pub fn read(&self) -> Result<(toml::Table, Positions<'a>), Diagnostic> {
        let document = Document::parse(self.text).map_err(|error| self.syntax_error(&error))?;
        let spans = SpanIndex::harvest(&document);
        let table = toml::Table::deserialize(toml_edit::de::Deserializer::from(document))
            .map_err(|error| self.syntax_error(&error))?;
        Ok((
            table,
            Positions {
                source: self.clone(),
                spans,
            },
        ))
    }

    /// The document an editor changes in place.
    pub fn parse_mut(&self) -> Result<DocumentMut, Diagnostic> {
        Ok(Document::parse(self.text)
            .map_err(|error| self.syntax_error(&error))?
            .into_mut())
    }

    /// The start of the file: the empty span at byte 0, line 1, column 1.
    ///
    /// What a diagnostic about the file as a whole points at. The empty
    /// *span* rather than [`Position::at`] is what makes such a diagnostic
    /// sort first: [`crate::manifest::diagnostics::Report::sort_by_position`]
    /// orders by span start, and a position without one sorts among the
    /// unpositioned.
    pub fn start(&self) -> Position {
        self.position(0..0)
    }

    /// A file that is not TOML at all, said the same way wherever it is
    /// found: a file that cannot be parsed must not be described one way
    /// when it is read and another when it is edited.
    fn syntax_error(&self, error: &impl TomlSyntaxError) -> Diagnostic {
        Diagnostic::error(format!("`{}` is not valid TOML: {error}", self.file))
            .at_opt(error.span().map(|span| self.position(span)))
    }

    /// The position of `span.start`: its 1-based line and character
    /// column.
    fn position(&self, span: Range<usize>) -> Position {
        assert!(
            self.text.is_char_boundary(span.start),
            "offset {} is not a character boundary of the text it was parsed from",
            span.start
        );
        let before = &self.text[..span.start];
        let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = before
            .rsplit_once('\n')
            .map_or(before, |(_, last)| last)
            .chars()
            .count()
            + 1;
        Position {
            file: self.file.clone(),
            line,
            column,
            span: Some(span),
        }
    }
}

/// What the two parse failures of [`Source`] have in common: a message
/// and, usually, where in the text it happened.
trait TomlSyntaxError: std::fmt::Display {
    fn span(&self) -> Option<Range<usize>>;
}

impl TomlSyntaxError for toml_edit::TomlError {
    // Not recursion: `Self::span` is the inherent method of
    // `toml_edit::TomlError`, which shadows the trait's.
    fn span(&self) -> Option<Range<usize>> {
        Self::span(self)
    }
}

impl TomlSyntaxError for toml_edit::de::Error {
    fn span(&self) -> Option<Range<usize>> {
        Self::span(self)
    }
}

/// A [`SpanIndex`] together with the [`Source`] it was harvested from.
///
/// Borrowing the text rather than the document is what lets the document
/// be handed to the deserializer; nothing that escapes into a report
/// borrows anything.
#[derive(Clone, Debug)]
pub struct Positions<'a> {
    source: Source<'a>,
    spans: SpanIndex,
}

impl Positions<'_> {
    /// For a manifest that was not read from text -- one a command built
    /// itself, or a json project converted into the toml model. Every
    /// lookup yields `None`, so the validation code needs no second path
    /// for the case.
    pub fn without_spans(file: impl Into<String>) -> Positions<'static> {
        Positions {
            source: Source::new(file, ""),
            spans: SpanIndex::default(),
        }
    }

    pub fn file(&self) -> &str {
        self.source.file()
    }

    /// Where this key is, if it is in the document.
    pub fn at(&self, path: &[&str]) -> Option<Position> {
        let span = self.spans.get(path)?;
        Some(self.source.position(span.clone()))
    }

    /// Where this key is, or the innermost thing on its path that is in
    /// the document. This is what a diagnostic about a *missing* key
    /// uses.
    pub fn near(&self, path: &[&str]) -> Option<Position> {
        let span = self.spans.get_near(path)?;
        Some(self.source.position(span.clone()))
    }
}

#[cfg(test)]
#[path = "./spans_tests.rs"]
mod tests;
