// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::*;

#[test]
fn with_trailing_slash_adds_slash_when_missing() {
    let u = url::Url::parse("https://example.com/foo").unwrap();
    assert_eq!(with_trailing_slash(u).path(), "/foo/");
}

#[test]
fn with_trailing_slash_keeps_existing_slash() {
    let u = url::Url::parse("https://example.com/foo/").unwrap();
    assert_eq!(with_trailing_slash(u).path(), "/foo/");
}

#[test]
fn with_trailing_slash_sets_root_on_empty() {
    let u = url::Url::parse("https://example.com").unwrap();
    assert_eq!(with_trailing_slash(u).path(), "/");
}
