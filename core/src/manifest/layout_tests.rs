// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use toml_edit::{Array, DocumentMut};

use super::{Shape, apply, push, remove};

/// Edit the array `key = <array>` and return its text.
fn edited(array: &str, edit: impl FnOnce(&mut Array)) -> String {
    let mut doc: DocumentMut = format!("key = {array}\n").parse().expect("valid TOML");
    edit(doc["key"].as_array_mut().expect("an array"));
    let text = doc.to_string();
    text.parse::<DocumentMut>().expect("still valid TOML");
    text.strip_prefix("key = ")
        .and_then(|text| text.strip_suffix('\n'))
        .expect("the one key")
        .to_owned()
}

#[test]
fn one_per_line_lays_out_a_single_line_array() {
    assert_eq!(
        edited(r#"["a","b"]"#, |a| apply(a, Shape::OnePerLine)),
        r#"[
    "a",
    "b",
]"#
    );
}

#[test]
fn one_per_line_writes_an_empty_array_as_brackets() {
    assert_eq!(edited("[\n]", |a| apply(a, Shape::OnePerLine)), "[]");
}

#[test]
fn one_per_line_keeps_a_comment_in_an_empty_array() {
    assert_eq!(
        edited("[\n  # nothing yet\n]", |a| apply(a, Shape::OnePerLine)),
        "[\n    # nothing yet\n]"
    );
}

#[test]
fn one_per_line_reindents_and_keeps_every_comment() {
    let array = r#"[  # the list
  # about a
        "a",   # a's line
"b"  # b's line
    # before the bracket
      ]"#;
    assert_eq!(
        edited(array, |a| apply(a, Shape::OnePerLine)),
        r#"[  # the list
    # about a
    "a",   # a's line
    "b",  # b's line
    # before the bracket
]"#
    );
}

#[test]
fn one_per_line_keeps_a_comment_between_an_entry_and_its_comma() {
    assert_eq!(
        edited(
            r#"[
    "a"  # a's line
    ,
    "b",
]"#,
            |a| {
                apply(a, Shape::OnePerLine);
            }
        ),
        r#"[
    "a",  # a's line
    "b",
]"#
    );
}

#[test]
fn one_per_line_collapses_blank_lines_and_drops_them_at_the_brackets() {
    assert_eq!(
        edited(
            r#"[

    "a",


    # group b
    "b",

]"#,
            |a| {
                apply(a, Shape::OnePerLine);
            }
        ),
        r#"[
    "a",

    # group b
    "b",
]"#
    );
}

#[test]
fn applying_a_shape_twice_changes_nothing() {
    let array = r#"[  # the list
    "a",  # a

    # b
    "b",
    # end
]"#;
    let once = edited(array, |a| apply(a, Shape::OnePerLine));
    assert_eq!(edited(&once, |a| apply(a, Shape::OnePerLine)), once);
    assert_eq!(once, array);
}

#[test]
fn as_written_keeps_a_single_line_array_on_one_line() {
    assert_eq!(
        edited(r#"[ "a" ,"b" ]"#, |a| push(a, "c", Shape::AsWritten)),
        r#"["a", "b", "c"]"#
    );
}

#[test]
fn as_written_keeps_a_multi_line_array_one_per_line() {
    assert_eq!(
        edited("[\n  \"a\"\n]", |a| push(a, "b", Shape::AsWritten)),
        r#"[
    "a",
    "b",
]"#
    );
}

#[test]
fn pushing_onto_an_empty_array_follows_the_shape() {
    assert_eq!(edited("[]", |a| push(a, "a", Shape::AsWritten)), r#"["a"]"#);
    assert_eq!(
        edited("[]", |a| push(a, "a", Shape::OnePerLine)),
        "[\n    \"a\",\n]"
    );
}

#[test]
fn pushing_keeps_the_last_entrys_comment_on_its_line() {
    assert_eq!(
        edited("[\n    \"a\",  # a\n    # end\n]", |a| {
            push(a, "b", Shape::OnePerLine);
        }),
        r#"[
    "a",  # a
    "b",
    # end
]"#
    );
}

#[test]
fn removing_takes_the_comment_on_the_entrys_line_only() {
    let array = r#"[
    "a",  # a
    # about b
    "b",  # b
    # about c
    "c",
]"#;
    assert_eq!(
        edited(array, |a| remove(a, 1, Shape::OnePerLine)),
        r#"[
    "a",  # a
    # about b
    # about c
    "c",
]"#
    );
}

#[test]
fn removing_the_last_entry_keeps_the_comment_on_the_bracket_line() {
    assert_eq!(
        edited("[  # the list\n    \"a\",  # a\n]", |a| {
            remove(a, 0, Shape::OnePerLine);
        }),
        "[  # the list\n]"
    );
}

#[test]
fn removing_every_entry_leaves_empty_brackets() {
    assert_eq!(
        edited("[\n    \"a\",  # a\n]", |a| remove(a, 0, Shape::OnePerLine)),
        "[]"
    );
    assert_eq!(
        edited(r#"["a", "b"]"#, |a| remove(a, 1, Shape::AsWritten)),
        r#"["a"]"#
    );
}
