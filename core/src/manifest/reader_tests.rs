// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use toml::Spanned;
use toml::de::DeTable;

use crate::manifest::diagnostics::{Diagnostics, KeyLocation, Severity};
use crate::manifest::reader::TableReader;
use crate::manifest::spans::Source;

/// Parse a test document the way a reader does: once, keeping both the
/// values and where they came from.
fn parse(text: &'static str) -> (Spanned<DeTable<'static>>, Source<'static>) {
    let source = Source::new("model.toml", text);
    let table = source.read().expect("the test document parses");
    (table, source)
}

fn top<'a>(table: &'a Spanned<DeTable<'a>>, source: &'a Source<'a>) -> TableReader<'a> {
    TableReader::new(table.get_ref(), None, KeyLocation::TopLevel, source)
}

fn texts(report: &Diagnostics) -> Vec<&str> {
    report
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

fn places(report: &Diagnostics) -> Vec<(usize, usize)> {
    report
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            let position = diagnostic.position.as_ref().expect("positioned");
            (position.line, position.column)
        })
        .collect()
}

/// The one conversion the whole module rests on: a parsed document
/// keeps every value Sysand judges, each with where it came from. Every
/// TOML type arrives as itself, a datetime included.
#[test]
fn a_document_keeps_its_values_and_their_spans() {
    const TEXT: &str = "\
manifest-version = \"1.0\"
count = 3
when = 2026-01-02

[workspace]
members = [\"a\"]
";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let reader = top(&table, &source);
    assert_eq!(reader.string("manifest-version", &mut report), Some("1.0"));
    assert_eq!(
        reader.get("count").map(|value| value.get_ref().type_str()),
        Some("integer")
    );
    assert_eq!(
        reader.get("when").map(|value| value.get_ref().type_str()),
        Some("datetime")
    );
    let workspace = reader.table("workspace", &mut report).expect("a table");
    assert_eq!(workspace.strings("members", &mut report), Some(vec!["a"]));
    assert!(report.is_empty());
    let position = workspace.at_element("members", 0).expect("positioned");
    assert_eq!((position.line, position.column), (6, 12));
}

/// A file that is not TOML at all is the one failure that stops a read
/// before any rule runs. It still says where.
#[test]
fn a_syntax_error_is_one_positioned_diagnostic() {
    let diagnostic = Source::new("model.toml", "name = \n")
        .read()
        .expect_err("invalid TOML");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert!(
        diagnostic
            .message
            .starts_with("`model.toml` is not valid TOML"),
        "unexpected message: {}",
        diagnostic.message
    );
    let position = diagnostic.position.as_ref().expect("positioned");
    assert_eq!(position.line, 1);
}

#[test]
fn a_string_key_comes_back_as_written() {
    const TEXT: &str = "publisher = \"Acme Corp\"\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert_eq!(
        top(&table, &source).string("publisher", &mut report),
        Some("Acme Corp")
    );
    assert!(report.is_empty());
}

/// Whether a key is required is the caller's rule, not the reader's, so
/// an absent key is not a problem here.
#[test]
fn a_missing_key_is_not_a_diagnostic() {
    const TEXT: &str = "publisher = \"acme\"\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let reader = top(&table, &source);
    assert!(reader.string("absent", &mut report).is_none());
    assert!(reader.strings("absent", &mut report).is_none());
    assert!(reader.table("absent", &mut report).is_none());
    assert!(reader.tables("absent", &mut report).is_empty());
    assert!(report.is_empty());
    assert!(!reader.has("absent"));
    assert!(reader.has("publisher"));
}

#[test]
fn a_wrong_typed_key_is_one_positioned_diagnostic() {
    const TEXT: &str = "publisher = 1\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert!(
        top(&table, &source)
            .string("publisher", &mut report)
            .is_none()
    );
    assert_eq!(
        texts(&report),
        ["`publisher` in the top level must be a string, but it is an integer"]
    );
    assert_eq!(places(&report), [(1, 1)]);
}

/// One bad element does not cost the reader the rest of the array, and
/// each is reported where it is.
#[test]
fn every_bad_element_of_an_array_is_reported() {
    const TEXT: &str = "topics = [\"a\", 2, \"c\", false]\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert!(
        top(&table, &source)
            .strings("topics", &mut report)
            .is_none()
    );
    assert_eq!(
        texts(&report),
        [
            "entry 1 of `topics` in the top level must be a string, but it is an integer",
            "entry 3 of `topics` in the top level must be a string, but it is a boolean",
        ]
    );
}

#[test]
fn a_key_that_is_not_an_array_is_one_diagnostic() {
    const TEXT: &str = "topics = \"a\"\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert!(
        top(&table, &source)
            .strings("topics", &mut report)
            .is_none()
    );
    assert_eq!(
        texts(&report),
        ["`topics` in the top level must be an array of strings, but it is a string"]
    );
}

#[test]
fn an_array_of_tables_yields_its_entries() {
    const TEXT: &str = "\
[[inline-usages]]
name = \"a\"

[[inline-usages]]
name = \"b\"
";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let entries = top(&table, &source).tables("inline-usages", &mut report);
    assert!(report.is_empty());
    let names: Vec<Option<&str>> = entries
        .iter()
        .map(|entry| entry.string("name", &mut report))
        .collect();
    assert_eq!(names, [Some("a"), Some("b")]);
    assert_eq!(
        entries[1].location(),
        &KeyLocation::ArrayTable("inline-usages".to_owned())
    );
}

#[test]
fn an_array_element_that_is_not_a_table_is_one_diagnostic() {
    const TEXT: &str = "inline-usages = [{ name = \"a\" }, \"b\"]\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let entries = top(&table, &source).tables("inline-usages", &mut report);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        texts(&report),
        ["entry 1 of `inline-usages` in the top level must be a table, but it is a string"]
    );
}

/// `[workspace.group.<name>]` is a table of tables keyed by name, not an
/// array, so the names are what a reader gets back.
#[test]
fn a_table_of_tables_yields_its_entries_by_name() {
    const TEXT: &str = "\
[group.legacy]
publisher = \"old\"

[group.current]
publisher = \"new\"
";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let groups = top(&table, &source).named_tables("group", &mut report);
    assert!(report.is_empty());
    let names: Vec<&str> = groups.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["legacy", "current"]);
    assert_eq!(
        groups[0].1.location(),
        &KeyLocation::Table("group.legacy".to_owned())
    );
}

#[test]
fn a_named_table_that_is_not_a_table_is_one_diagnostic() {
    const TEXT: &str = "group = { legacy = 3 }\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let groups = top(&table, &source).named_tables("group", &mut report);
    assert!(groups.is_empty());
    assert_eq!(
        texts(&report),
        ["`legacy` in `[group]` must be a table, but it is an integer"]
    );
}

/// The known keys are the ones the reader asks for; everything else is
/// unknown, and each warning points at its own key.
#[test]
fn unknown_keys_are_what_is_left_over() {
    const TEXT: &str = "\
publisher = \"acme\"
colour = \"red\"
shape = \"round\"
";
    let (table, source) = parse(TEXT);
    let warnings = top(&table, &source).unknown_keys(&["publisher"], None);
    let messages: Vec<&str> = warnings
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "unknown top-level key `colour`",
            "unknown top-level key `shape`",
        ]
    );
    assert!(warnings.iter().all(|w| w.severity == Severity::Warning));
    let places: Vec<(usize, usize)> = warnings
        .iter()
        .map(|w| {
            let p = w.position.as_ref().expect("positioned");
            (p.line, p.column)
        })
        .collect();
    assert_eq!(places, [(2, 1), (3, 1)]);
}

/// Inheritance goes in one direction only: a marker where a plain value
/// belongs is an error.
#[test]
fn a_marker_where_a_value_belongs_says_so() {
    const TEXT: &str = "publisher = { workspace = true }\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert!(
        top(&table, &source)
            .string("publisher", &mut report)
            .is_none()
    );
    assert_eq!(
        texts(&report),
        [
            "`publisher` in the top level is an inheritance marker, but a workspace has \
             nothing to inherit from; give the value directly"
        ]
    );
}

/// `urls.workspace = true` is the whole-table marker form; a table-valued
/// key must not report it as an unknown key inside `urls`.
#[test]
fn a_table_valued_key_refuses_a_marker_too() {
    const TEXT: &str = "urls.workspace = true\n";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    assert!(top(&table, &source).table("urls", &mut report).is_none());
    assert_eq!(
        texts(&report),
        [
            "`urls` in the top level is an inheritance marker, but a workspace has \
             nothing to inherit from; give the value directly"
        ]
    );
}

/// Every message a nested reader produces names the table it is about.
#[test]
fn a_table_reader_names_its_table() {
    const TEXT: &str = "\
[workspace.group.legacy]
publisher = 1
";
    let (table, source) = parse(TEXT);
    let mut report = Diagnostics::new();
    let reader = top(&table, &source)
        .table("workspace", &mut report)
        .expect("a table")
        .table("group", &mut report)
        .expect("a table")
        .table("legacy", &mut report)
        .expect("a table");
    assert_eq!(
        reader.location(),
        &KeyLocation::Table("workspace.group.legacy".to_owned())
    );
    assert!(reader.string("publisher", &mut report).is_none());
    assert_eq!(
        texts(&report),
        ["`publisher` in `[workspace.group.legacy]` must be a string, but it is an integer"]
    );
    assert_eq!(places(&report), [(2, 1)]);
}
