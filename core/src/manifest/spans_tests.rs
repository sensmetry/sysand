// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use toml::Spanned;
use toml::de::DeTable;

use crate::manifest::diagnostics::{Diagnostics, KeyLocation, Position};
use crate::manifest::reader::TableReader;
use crate::manifest::spans::Source;

/// Line 1 `manifest-version`, 3 `[project]`, 4 `name`, 5 `urls`,
/// 7 and 11 `[[inline-usages]]`, 8 and 12 `publisher`, 9 `name`.
const DOC: &str = "\
manifest-version = \"1.0\"

[project]
name = \"app\"
urls = { website = \"https://example.com\" }

[[inline-usages]]
publisher = \"acme\"
name = \"lib\"

[[inline-usages]]
publisher = \"acme\"
";

fn parse(text: &str) -> (Spanned<DeTable<'_>>, Source<'_>) {
    let source = Source::new("model.toml", text);
    let table = source.read().expect("the test document parses");
    (table, source)
}

fn top<'a>(table: &'a Spanned<DeTable<'a>>, source: &'a Source<'a>) -> TableReader<'a> {
    TableReader::new(table.get_ref(), None, KeyLocation::TopLevel, source)
}

fn table<'a>(reader: &TableReader<'a>, key: &str) -> TableReader<'a> {
    reader.table(key, &mut Diagnostics::new()).expect("a table")
}

fn place(position: Option<Position>) -> (usize, usize) {
    let position = position.expect("positioned");
    (position.line, position.column)
}

/// A key that is not in the document is where its table is, and the
/// document itself is nowhere.
#[test]
fn top_level_keys_are_where_they_are_written() {
    let (document, source) = parse(DOC);
    let top = top(&document, &source);
    let position = top.at("manifest-version").expect("positioned");
    assert_eq!((position.line, position.column), (1, 1));
    assert!(top.at("not-a-key").is_none());
}

/// A table's own key is what a diagnostic about the table as a whole
/// points at.
#[test]
fn a_table_is_where_its_key_is() {
    let (document, source) = parse(DOC);
    let project = table(&top(&document, &source), "project");
    assert_eq!(place(project.here()), (3, 2));
    assert_eq!(place(project.at("name")), (4, 1));
}

/// Two entries of one array are told apart, so a diagnostic about the
/// second one does not point at the first.
#[test]
fn each_entry_of_an_array_of_tables_is_where_it_is() {
    let (document, source) = parse(DOC);
    let entries = top(&document, &source).tables("inline-usages", &mut Diagnostics::new());
    assert_eq!(place(entries[0].at("publisher")), (8, 1));
    assert_eq!(place(entries[1].at("publisher")), (12, 1));
    // The entry as a whole is its `[[inline-usages]]` header, which is
    // what a diagnostic about a key the entry does not have points at.
    assert_eq!(place(entries[1].here()), (11, 1));
    assert_eq!(place(entries[1].at("name")), (11, 1));
}

/// `urls = { website = "..." }` and `[project.urls]` are read the same
/// way.
#[test]
fn an_inline_table_is_read_like_a_table() {
    let (document, source) = parse(DOC);
    let urls = table(&table(&top(&document, &source), "project"), "urls");
    assert_eq!(place(urls.at("website")), (5, 10));
}

/// The third spelling of the same table.
#[test]
fn a_dotted_key_is_where_each_of_its_parts_is() {
    let text = "[project]\nurls.website = \"https://example.com\"\n";
    let (document, source) = parse(text);
    let urls = table(&table(&top(&document, &source), "project"), "urls");
    assert_eq!(place(urls.here()), (2, 1));
    assert_eq!(place(urls.at("website")), (2, 6));
}

/// TOML lets a quoted key contain a dot, so `"a.b"` and `[a]` + `b` are
/// different keys that may both appear in one file.
#[test]
fn a_quoted_dotted_key_does_not_collide_with_a_table() {
    let text = "\"a.b\" = 1\n\n[a]\nb = 2\n";
    let (document, source) = parse(text);
    let top = top(&document, &source);
    assert_eq!(place(top.at("a.b")), (1, 1));
    assert_eq!(place(table(&top, "a").at("b")), (4, 1));
}

/// `members`, `exclude`, `topics` and `maintainers` are arrays of
/// strings, and a diagnostic about one entry points at the entry, not at
/// the key that holds the array, unless the entry is not there.
#[test]
fn each_element_of_an_array_is_where_it_is() {
    const TEXT: &str = "\
[workspace]
members = [\"a\", \"b\"]
";
    let (document, source) = parse(TEXT);
    let workspace = table(&top(&document, &source), "workspace");
    assert_eq!(place(workspace.at_element("members", 0)), (2, 12));
    assert_eq!(place(workspace.at_element("members", 1)), (2, 17));
    assert_eq!(place(workspace.at_element("members", 2)), (2, 1));
}

/// The same array written inline or as `[[...]]` headers is the same
/// array; an inline entry is where its `{ ... }` is.
#[test]
fn an_inline_array_of_tables_is_read_like_an_array_of_tables() {
    const TEXT: &str = "inline-usages = [{ name = \"x\" }]\n";
    let (document, source) = parse(TEXT);
    let entries = top(&document, &source).tables("inline-usages", &mut Diagnostics::new());
    assert_eq!(place(entries[0].here()), (1, 18));
    assert_eq!(place(entries[0].at("name")), (1, 20));
}

/// The parser keeps a number as its text, so a number TOML cannot
/// represent is refused after it, as invalid TOML, wherever it is, in the
/// words the editor refuses it with.
#[test]
fn a_number_toml_cannot_represent_is_invalid_toml() {
    for (text, place) in [
        ("a = 99999999999999999999\n", (1, 5)),
        ("a = [1, { b = 0x1_0000_0000_0000_0000 }]\n", (1, 15)),
        ("a = 1e999\n", (1, 5)),
    ] {
        let source = Source::new("model.toml", text);
        let diagnostic = source.read().expect_err("not representable");
        assert!(
            diagnostic
                .message
                .starts_with("`model.toml` is not valid TOML"),
            "unexpected message: {}",
            diagnostic.message
        );
        let editing = source.parse_mut().expect_err("not representable");
        assert_eq!(diagnostic, editing, "{text}");
        let position = diagnostic.position.expect("positioned");
        assert_eq!((position.line, position.column), place, "{text}");
    }
    for text in ["a = 9223372036854775807\n", "a = -inf\n", "a = 1e308\n"] {
        Source::new("model.toml", text)
            .read()
            .expect("representable");
    }
}

/// What a message quotes is the text exactly as written.
#[test]
fn a_span_is_spelled_as_written() {
    let text = "a = [ 1,2 ]\n";
    let (document, source) = parse(text);
    let value = document.get_ref().get("a").expect("written");
    assert_eq!(source.spelled(value.span()), "[ 1,2 ]");
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
