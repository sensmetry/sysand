// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The layout of the arrays a `model.toml` writer generates or edits.
//!
//! An edited array is re-laid out whole, in its key's [`Shape`], keeping
//! every comment in it. Nothing outside the array is touched.
//!
//! `toml_edit` stores what sits between two entries in two places: the
//! suffix of the entry before the comma and the prefix of the entry after
//! it. A comment written at the end of one entry's line therefore lives at
//! the front of the *next* entry's prefix, or in the array's trailing for
//! the last entry. [`Gap`] reads the two back as lines, so that a comment
//! stays with the entry whose line it is on.

use toml_edit::{Array, Value};

/// How an array is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One entry per line, indented by [`INDENT`], with a trailing comma
    /// and the closing bracket at the start of its own line. An empty
    /// array with no comments is `[]`.
    OnePerLine,
    /// `["a", "b"]` while the array fits that form, [`Shape::OnePerLine`]
    /// once it spans lines or holds a comment.
    AsWritten,
}

/// The indentation of an entry in a [`Shape::OnePerLine`] array.
pub const INDENT: &str = "    ";

/// Lay `array` out in `shape`.
pub fn apply(array: &mut Array, shape: Shape) {
    let gaps = Gap::read_all(array);
    write(array, shape, &gaps);
}

/// Append an entry and lay the array out in `shape`.
///
/// Comments written before the closing bracket stay before it.
pub fn push(array: &mut Array, entry: impl Into<Value>, shape: Shape) {
    let mut gaps = Gap::read_all(array);
    let before_bracket = gaps.pop().unwrap_or_default();
    gaps.push(Gap {
        end_of_line: before_bracket.end_of_line,
        lines: Vec::new(),
        line_break: before_bracket.line_break,
    });
    gaps.push(Gap {
        end_of_line: None,
        lines: before_bracket.lines,
        line_break: before_bracket.line_break,
    });
    array.push(entry);
    write(array, shape, &gaps);
}

/// Remove the entry at `at` and lay the array out in `shape`.
///
/// What goes is what was on the entry's own line: the entry and the
/// comment at the end of that line. Comment lines above and below it stay.
///
/// # Panics
///
/// If `at` is out of range.
pub fn remove(array: &mut Array, at: usize, shape: Shape) {
    let mut gaps = Gap::read_all(array);
    let after = gaps.remove(at + 1);
    gaps[at].lines.extend(after.lines);
    array.remove(at);
    write(array, shape, &gaps);
}

/// What sits between two entries, between `[` and the first entry, or
/// between the last entry and `]`, read as lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Gap {
    /// A comment at the end of the line before the gap, with the
    /// whitespace written before it.
    end_of_line: Option<String>,
    /// The lines wholly inside the gap.
    lines: Vec<Line>,
    /// Whether the gap holds a newline.
    line_break: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    Blank,
    Comment(String),
}

impl Gap {
    /// One gap per entry, before it, and one before `]`.
    fn read_all(array: &Array) -> Vec<Self> {
        fn raw(text: Option<&toml_edit::RawString>) -> &str {
            text.and_then(toml_edit::RawString::as_str)
                .unwrap_or_default()
        }
        let mut gaps = Vec::with_capacity(array.len() + 1);
        let mut before_comma = None;
        for item in array {
            gaps.push(Self::read(before_comma, raw(item.decor().prefix())));
            before_comma = Some(raw(item.decor().suffix()));
        }
        let trailing = array.trailing().as_str().unwrap_or_default();
        gaps.push(Self::read(before_comma, trailing));
        gaps
    }

    /// Read the text between two entries: `before_comma`, the suffix of
    /// the entry before (`None` for the gap after `[`), and `after_comma`.
    ///
    /// A comment ends at a newline, so every comment is either on the line
    /// before the gap or on a line of its own. The text up to the first
    /// newline is on the line before the gap, and the text after the last
    /// newline is the indentation of what follows the gap. A line that
    /// held only the comma is dropped.
    fn read(before_comma: Option<&str>, after_comma: &str) -> Self {
        let mut segments: Vec<(&str, bool)> = Vec::new();
        let mut after = after_comma.split('\n');
        if let Some(before) = before_comma {
            segments.extend(before.split('\n').map(|segment| (segment, false)));
            // The last segment before the comma is whitespace, and shares
            // its line with the first one after it.
            segments.pop();
            segments.push((after.next().unwrap_or_default(), true));
        }
        segments.extend(after.map(|segment| (segment, false)));

        let line_break = segments.len() > 1;
        let head = segments
            .first()
            .map_or_default(|(segment, _)| segment.trim_end());
        let end_of_line = head.contains('#').then(|| head.to_owned());
        let inner = segments
            .get(1..segments.len().saturating_sub(1))
            .unwrap_or_default();
        let lines = inner
            .iter()
            .filter_map(|&(segment, comma_line)| match segment.trim() {
                "" if comma_line => None,
                "" => Some(Line::Blank),
                comment => Some(Line::Comment(comment.to_owned())),
            })
            .collect();
        Self {
            end_of_line,
            lines,
            line_break,
        }
    }

    fn has_comments(&self) -> bool {
        self.end_of_line.is_some() || self.lines.iter().any(|line| *line != Line::Blank)
    }

    /// The gap as written in a one-per-line array, followed by `indent`.
    ///
    /// Runs of blank lines become one, and blank lines next to a bracket
    /// are dropped.
    fn render(&self, first: bool, last: bool, indent: &str) -> String {
        let mut text = String::new();
        if let Some(comment) = &self.end_of_line {
            if !comment.starts_with(char::is_whitespace) {
                text.push(' ');
            }
            text.push_str(comment);
        }
        text.push('\n');

        let mut blank = false;
        let mut written = false;
        for line in &self.lines {
            match line {
                Line::Blank => blank = true,
                Line::Comment(comment) => {
                    if blank && (written || !first) {
                        text.push('\n');
                    }
                    text.push_str(INDENT);
                    text.push_str(comment);
                    text.push('\n');
                    blank = false;
                    written = true;
                }
            }
        }
        if blank && !last && (written || !first) {
            text.push('\n');
        }
        text.push_str(indent);
        text
    }
}

fn write(array: &mut Array, shape: Shape, gaps: &[Gap]) {
    if shape == Shape::AsWritten && !gaps.iter().any(|gap| gap.line_break) {
        one_line(array);
    } else if array.is_empty() && !gaps.iter().any(Gap::has_comments) {
        array.set_trailing("");
        array.set_trailing_comma(false);
    } else {
        one_per_line(array, gaps);
    }
}

fn one_line(array: &mut Array) {
    for (at, item) in array.iter_mut().enumerate() {
        let decor = item.decor_mut();
        decor.set_prefix(if at == 0 { "" } else { " " });
        decor.set_suffix("");
    }
    array.set_trailing("");
    array.set_trailing_comma(false);
}

fn one_per_line(array: &mut Array, gaps: &[Gap]) {
    let len = array.len();
    for (at, item) in array.iter_mut().enumerate() {
        let decor = item.decor_mut();
        decor.set_prefix(gaps[at].render(at == 0, false, INDENT));
        decor.set_suffix("");
    }
    array.set_trailing(gaps[len].render(len == 0, true, ""));
    array.set_trailing_comma(len > 0);
}

#[cfg(test)]
#[path = "./layout_tests.rs"]
mod tests;
