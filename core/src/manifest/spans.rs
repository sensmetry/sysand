// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! A `model.toml` read as TOML, and the positions its diagnostics point
//! at.
//!
//! [`Source::read`] parses the text into a [`DeTable`]: every key and
//! every value carries its own byte span, and strings borrow from the
//! text, so a reader holds the span of whatever it is looking at and
//! nothing is copied or indexed up front. A span becomes a [`Position`]
//! (a file, a line and a column) only when a diagnostic needs one, through
//! [`Source::position`].

use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};
use toml_edit::DocumentMut;

use crate::manifest::diagnostics::{Diagnostic, Position};

/// A file read as TOML, under the name its messages call it by.
///
/// The only thing that turns a byte span into a [`Position`]: every span
/// it turns is one that parsing its own text produced, so a span always
/// falls on a character of the text it is measured against.
#[derive(Clone, Debug)]
pub struct Source<'a> {
    file: &'a str,
    text: &'a str,
}

impl<'a> Source<'a> {
    pub fn new(file: &'a str, text: &'a str) -> Self {
        Self { file, text }
    }

    pub fn file(&self) -> &'a str {
        self.file
    }

    /// The document, every key and value with its span.
    ///
    /// The `Err` is the one failure that stops a read before any rule of
    /// the manifest runs: the file is not TOML at all.
    pub fn read(&self) -> Result<Spanned<DeTable<'a>>, Diagnostic> {
        let document =
            DeTable::parse(self.text).map_err(|error| self.syntax_error(&error, error.span()))?;
        match unrepresentable(document.get_ref()) {
            None => Ok(document),
            // The editing parser's refusal of such a number is preferred.
            Some(span) => Err(self
                .parse_mut()
                .err()
                .unwrap_or_else(|| self.syntax_error(&"the number is out of range", Some(span)))),
        }
    }

    /// The document an editor changes in place.
    pub fn parse_mut(&self) -> Result<DocumentMut, Diagnostic> {
        self.text
            .parse()
            .map_err(|error: toml_edit::TomlError| self.syntax_error(&error, error.span()))
    }

    /// The text of a span, exactly as the file writes it.
    pub fn spelled(&self, span: Range<usize>) -> &'a str {
        &self.text[span]
    }

    /// The start of the file: the empty span at byte 0, line 1, column 1.
    ///
    /// What a diagnostic about the file as a whole points at. The empty
    /// span makes such a diagnostic sort first, as
    /// [`crate::manifest::diagnostics::Diagnostics::sort_by_position`]
    /// orders by span start.
    pub fn start(&self) -> Position {
        self.position(0..0)
    }

    /// The position of `span.start`: its 1-based line and character
    /// column.
    pub fn position(&self, span: Range<usize>) -> Position {
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
            line,
            column,
            span: Some(span),
        }
    }

    /// A file that is not TOML at all, said the same way wherever it is
    /// found.
    fn syntax_error(
        &self,
        error: &impl std::fmt::Display,
        span: Option<Range<usize>>,
    ) -> Diagnostic {
        Diagnostic::error(format!("`{}` is not valid TOML: {error}", self.file))
            .at_opt(span.map(|span| self.position(span)))
    }
}

/// The first number that TOML cannot represent, if there is one.
///
/// The parser keeps a number as the text that spells it, and a number
/// that does not fit (an integer outside 64 bits, a float that overflows
/// to infinity) makes the file invalid, wherever it is.
fn unrepresentable(table: &DeTable<'_>) -> Option<Range<usize>> {
    table.values().find_map(unrepresentable_value)
}

fn unrepresentable_value(value: &Spanned<DeValue<'_>>) -> Option<Range<usize>> {
    match value.get_ref() {
        DeValue::Integer(integer) => i64::from_str_radix(integer.as_str(), integer.radix())
            .is_err()
            .then(|| value.span()),
        DeValue::Float(float) => (float.as_str().parse::<f64>().is_ok_and(f64::is_infinite)
            && !float.as_str().contains("inf"))
        .then(|| value.span()),
        DeValue::Table(table) => unrepresentable(table),
        DeValue::Array(elements) => elements.iter().find_map(unrepresentable_value),
        DeValue::String(_) | DeValue::Boolean(_) | DeValue::Datetime(_) => None,
    }
}

#[cfg(test)]
#[path = "./spans_tests.rs"]
mod tests;
