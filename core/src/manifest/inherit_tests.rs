// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::manifest::common::InheritFrom;
use crate::manifest::diagnostics::{KeyLocation, Report};
use crate::manifest::inherit::{MetaProperty, inheritable, inheritable_table, own};
use crate::manifest::reader::TableReader;
use crate::manifest::spans::Positions;
use crate::manifest::spans::Source;

/// A document read the way `[project]` is read, but at the top level, so
/// that a test writes one key and nothing else.
fn parse(text: &'static str) -> (toml::Table, Positions<'static>) {
    Source::new("model.toml", text)
        .read()
        .expect("the test document parses")
}

fn top<'a>(table: &'a toml::Table, positions: &'a Positions<'a>) -> TableReader<'a> {
    TableReader::new(table, Vec::new(), KeyLocation::TopLevel, positions)
}

fn texts(report: &Report) -> Vec<&str> {
    report
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect()
}

/// Read one scalar key that may be inherited, as `[project]` reads
/// `publisher`.
fn scalar(text: &'static str) -> (Option<MetaProperty<String>>, Report) {
    let (table, positions) = parse(text);
    let reader = top(&table, &positions);
    let mut report = Report::new();
    let value = inheritable(&reader, "publisher", &mut report, |report| {
        reader.string("publisher", report).map(ToOwned::to_owned)
    });
    (value, report)
}

/// Read one key that is not inheritable, as `[project]` reads `sources`.
fn refused(text: &'static str) -> Report {
    let (table, positions) = parse(text);
    let reader = top(&table, &positions);
    let mut report = Report::new();
    let value = own(
        &reader,
        "sources",
        "every member writes out its own source files in full",
        &mut report,
        |report| reader.strings("sources", report),
    );
    assert!(value.is_none(), "a refused key yields nothing");
    report
}

#[test]
fn a_plain_value_is_provided() {
    let (value, report) = scalar("publisher = \"acme\"\n");
    assert!(report.is_empty());
    match value.expect("a value") {
        MetaProperty::Provided(publisher) => assert_eq!(publisher, "acme"),
        MetaProperty::Inherited(from) => panic!("unexpected marker: {from:?}"),
    }
}

/// workspaces.md "Explicit inheritance": "to inherit a key from the
/// default group, `<key>.workspace = true` is used".
#[test]
fn a_default_group_marker_is_inherited() {
    let (value, report) = scalar("publisher.workspace = true\n");
    assert!(report.is_empty());
    match value.expect("a value") {
        MetaProperty::Inherited(inherited) => {
            assert_eq!(inherited.from, InheritFrom::DefaultGroup);
        }
        MetaProperty::Provided(value) => panic!("unexpected value: {value}"),
    }
}

/// "To inherit from a non-default group, `<key>.workspace =
/// \"group-name\"`."
#[test]
fn a_named_group_marker_is_inherited() {
    let (value, report) = scalar("publisher.workspace = \"legacy\"\n");
    assert!(report.is_empty());
    match value.expect("a value") {
        MetaProperty::Inherited(inherited) => {
            assert_eq!(inherited.from, InheritFrom::Group("legacy".to_owned()));
        }
        MetaProperty::Provided(value) => panic!("unexpected value: {value}"),
    }
}

/// There is no "inherit nothing" marker.
#[test]
fn workspace_false_is_not_a_marker() {
    let (value, report) = scalar("publisher.workspace = false\n");
    assert!(value.is_none());
    assert_eq!(
        texts(&report),
        ["`workspace = false` is not allowed; a key that is not inherited carries its own value"]
    );
}

/// workspaces.md: "`<group-name>` is a bare TOML key: letters, digits,
/// `-` and `_`."
#[test]
fn a_marker_naming_an_invalid_group_is_refused() {
    let (value, report) = scalar("publisher.workspace = \"a.b\"\n");
    assert!(value.is_none());
    assert_eq!(
        texts(&report),
        [
            "`a.b` is not a valid workspace group name (a bare TOML key: letters, digits, \
             `-` and `_`)"
        ]
    );
}

/// A marker names a group or selects the default one; nothing else can
/// stand where its value does.
#[test]
fn a_marker_with_a_non_marker_value_is_refused() {
    let (value, report) = scalar("publisher.workspace = 3\n");
    assert!(value.is_none());
    assert_eq!(
        texts(&report),
        [
            "`publisher.workspace` in the top level must be a boolean or a string, but it is an integer"
        ]
    );
}

/// workspaces.md: "inheritance cannot overlap: when a key is inherited
/// from a table and the key's value is also a table, inheritance cannot
/// be specified for any of the inner table's keys". Such a table is not
/// a marker (it has more than one key), so without this check the
/// `workspace` key would come back as an unknown key inside `urls` --
/// a warning, with the inheritance silently dropped.
#[test]
fn an_overlapping_marker_is_refused() {
    for inner in [
        "urls.website.workspace = true",
        "urls.website.workspace = \"legacy\"",
        "urls.website = \"https://example.com\"",
    ] {
        let text = format!("urls.workspace = true\n{inner}\n");
        let (table, positions) = Source::new("model.toml", &text).read().expect("parses");
        let reader = top(&table, &positions);
        let mut report = Report::new();
        let value = inheritable_table(&reader, "urls", &mut report, |report| {
            reader.table("urls", report).map(|_| ())
        });
        assert!(value.is_none(), "{inner}");
        assert_eq!(
            texts(&report),
            [
                "inheritance cannot overlap: `urls` in the top level carries a `workspace` \
                 marker and also sets `website`;\na key is either inherited or written out in full"
            ],
            "{inner}"
        );
    }
}

/// The rule is stated for a key "whose value is also a table". A scalar
/// key cannot take a table at all, so a `workspace` key among others is
/// an ordinary type error, not an overlap.
#[test]
fn a_scalar_key_with_a_workspace_key_among_others_is_a_type_error() {
    let (value, report) = scalar("publisher = { workspace = true, x = 1 }\n");
    assert!(value.is_none());
    assert_eq!(
        texts(&report),
        ["`publisher` in the top level must be a string, but it is a table"]
    );
}

/// workspaces.md "Common metadata": the keys that are deliberately not
/// inheritable say so, rather than reporting a table where an array was
/// expected.
#[test]
fn a_non_inheritable_key_refuses_a_marker() {
    assert_eq!(
        texts(&refused("sources.workspace = true\n")),
        [
            "`sources` in the top level cannot be inherited from the workspace;\n\
             every member writes out its own source files in full"
        ]
    );
}

/// `workspace = false` is not a marker either, but for a key that can
/// never be inherited the useful thing to say is still that it cannot be
/// inherited -- not that there is no "inherit nothing" marker.
#[test]
fn a_non_inheritable_key_refuses_a_false_marker_too() {
    let report = refused("sources.workspace = false\n");
    assert_eq!(texts(&report).len(), 1);
    assert!(
        texts(&report)[0].contains("cannot be inherited"),
        "unexpected message: {}",
        texts(&report)[0]
    );
}

/// The marker carries where it was written, so that a diagnostic raised
/// much later -- when the workspace is known -- can still point at it.
#[test]
fn a_marker_carries_its_own_position() {
    let (value, report) = scalar("# a comment\n\npublisher.workspace = true\n");
    assert!(report.is_empty());
    match value.expect("a value") {
        MetaProperty::Inherited(inherited) => {
            let position = inherited.at.expect("positioned");
            assert_eq!((position.line, position.column), (3, 1));
            assert_eq!(position.file, "model.toml");
        }
        MetaProperty::Provided(value) => panic!("unexpected value: {value}"),
    }
}
