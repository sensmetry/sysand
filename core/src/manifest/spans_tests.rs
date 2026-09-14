// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::spans::{Positions, Source};

/// Line 1 `manifest-version`, 3 `[project]`, 4 `name`, 5 `urls`,
/// 7 and 11 `[[inline-usages]]`, 8 and 12 `publisher`, 9 `name`.
const DOC: &str = "\
manifest-version = 1

[project]
name = \"app\"
urls = { website = \"https://example.com\" }

[[inline-usages]]
publisher = \"acme\"
name = \"lib\"

[[inline-usages]]
publisher = \"acme\"
";

fn positions(text: &str) -> Positions<'_> {
    Source::new("model.toml", text)
        .read()
        .expect("the test document parses")
        .1
}

/// Every diagnostic carries the file it is about, and a key that is not
/// in the document has no position rather than a made-up one.
#[test]
fn top_level_keys_are_indexed() {
    let positions = positions(DOC);
    let position = positions.at(&["manifest-version"]).expect("indexed");
    assert_eq!(position.file, "model.toml");
    assert_eq!((position.line, position.column), (1, 1));
    assert!(positions.at(&["not-a-key"]).is_none());
}

/// A table's own key is what a diagnostic about the table as a whole
/// points at; its keys are addressed below it.
#[test]
fn table_keys_are_indexed_by_their_path() {
    let positions = positions(DOC);
    let table = positions.at(&["project"]).expect("indexed");
    assert_eq!((table.line, table.column), (3, 2));
    let name = positions.at(&["project", "name"]).expect("indexed");
    assert_eq!((name.line, name.column), (4, 1));
}

/// Two entries of one array are told apart by their index, so a
/// diagnostic about the second one does not point at the first.
#[test]
fn array_of_tables_entries_are_indexed_by_position() {
    let positions = positions(DOC);
    let first = positions
        .at(&["inline-usages", "0", "publisher"])
        .expect("indexed");
    assert_eq!((first.line, first.column), (8, 1));
    let second = positions
        .at(&["inline-usages", "1", "publisher"])
        .expect("indexed");
    assert_eq!((second.line, second.column), (12, 1));
    // The entry as a whole is its `[[inline-usages]]` header, which is
    // what a diagnostic about a key the entry does not have points at.
    let entry = positions.at(&["inline-usages", "1"]).expect("indexed");
    assert_eq!((entry.line, entry.column), (11, 1));
}

/// `urls = { website = "..." }` and `[project.urls]` are the same table
/// written two ways, so a reader of the index cannot be made to care
/// which one the author chose.
#[test]
fn inline_tables_are_indexed_like_tables() {
    let positions = positions(DOC);
    let website = positions
        .at(&["project", "urls", "website"])
        .expect("indexed");
    assert_eq!((website.line, website.column), (5, 10));
}

/// The third spelling of the same table.
#[test]
fn dotted_keys_are_indexed_by_their_full_path() {
    let text = "[project]\nurls.website = \"https://example.com\"\n";
    let positions = positions(text);
    let website = positions
        .at(&["project", "urls", "website"])
        .expect("indexed");
    assert_eq!((website.line, website.column), (2, 6));
    let urls = positions.at(&["project", "urls"]).expect("indexed");
    assert_eq!((urls.line, urls.column), (2, 1));
}

/// TOML lets a quoted key contain a dot, so `"a.b"` and `[a]` + `b` are
/// different keys that may both appear in one file. A path of segments
/// keeps them apart; a single dotted string would not.
#[test]
fn a_quoted_dotted_key_does_not_collide_with_a_table() {
    let text = "\"a.b\" = 1\n\n[a]\nb = 2\n";
    let positions = positions(text);
    let quoted = positions.at(&["a.b"]).expect("indexed");
    assert_eq!((quoted.line, quoted.column), (1, 1));
    let nested = positions.at(&["a", "b"]).expect("indexed");
    assert_eq!((nested.line, nested.column), (4, 1));
}

/// A diagnostic about a key that is *missing* has no key to point at, so
/// it points at the innermost thing that is there. When nothing on the
/// path is there either -- a manifest that was not read from text -- it
/// has no position at all.
#[test]
fn near_falls_back_to_the_nearest_indexed_ancestor() {
    let positions = positions(DOC);
    // The second entry has no `name`.
    let missing = positions
        .near(&["inline-usages", "1", "name"])
        .expect("falls back to the entry");
    assert_eq!((missing.line, missing.column), (11, 1));
    // An exact hit is preferred over any ancestor.
    let present = positions
        .near(&["inline-usages", "0", "name"])
        .expect("indexed");
    assert_eq!((present.line, present.column), (9, 1));
    assert!(positions.near(&["nowhere", "0", "name"]).is_none());

    let empty = Positions::without_spans("model.toml");
    assert!(empty.at(&["project"]).is_none());
    assert!(empty.near(&["project", "name"]).is_none());
}

#[test]
fn a_position_resolves_line_and_column_from_a_span() {
    let text = "manifest-version = \"1.0\"\n\n[project]\nname = \"ä-project\"\nversion = 1\n";
    let source = Source::new("model.toml", text);
    let span = |needle: &str| {
        let start = text.find(needle).expect("needle is in the text");
        start..start + needle.len()
    };

    let start = source.position(span("manifest-version"));
    assert_eq!((start.line, start.column), (1, 1));

    let project = source.position(span("[project]"));
    assert_eq!((project.line, project.column), (3, 1));

    let version_line = span("\nversion = 1");
    let version = source.position(version_line.start + 1..version_line.end);
    assert_eq!((version.line, version.column), (5, 1));

    // The column counts characters, not bytes: `ä` is two bytes, so the
    // `-` that follows it is the tenth character of its line.
    let inside = source.position(span("-project"));
    assert_eq!((inside.line, inside.column), (4, 10));
    assert_eq!(inside.span, Some(span("-project")));

    let file = source.start();
    assert_eq!((file.line, file.column, file.span), (1, 1, Some(0..0)));
}
