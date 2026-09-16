// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::BTreeMap;

use super::{Difference, compare_at};
use serde_json::{Value, json};

/// The comparator's JSON half, without building an archive: the rules
/// under test are about values, and a KPAR would add nothing but IO.
fn differences(built: &Value, published: &Value, ignored: &[&str]) -> Vec<Difference> {
    let mut out = Vec::new();
    compare_at(".project.json", "", built, published, ignored, &mut out);
    out
}

#[test]
fn equal_projects_compare_equal() {
    let value = json!({"name": "dep", "version": "1.0.0"});
    assert_eq!(
        differences(&value, &value, &[]),
        [] as [crate::stdlib_compare::Difference; 0]
    );
}

#[test]
fn key_order_is_ignored() {
    let built = json!({"name": "dep", "version": "1.0.0"});
    let published = json!({"version": "1.0.0", "name": "dep"});
    assert_eq!(
        differences(&built, &published, &[]),
        [] as [crate::stdlib_compare::Difference; 0]
    );
}

/// The published constraints are bare `1.0.0`; a build writes `^1.0.0`.
/// They are the same requirement, and a string comparison would call
/// all ten libraries different.
#[test]
fn a_bare_constraint_equals_a_caret_constraint() {
    let built = json!({"usage": [{"resource": "x", "versionConstraint": "^1.0.0"}]});
    let published = json!({"usage": [{"resource": "x", "versionConstraint": "1.0.0"}]});
    assert!(
        differences(&built, &published, &[]).is_empty(),
        "{:?}",
        differences(&built, &published, &[])
    );
}

#[test]
fn a_different_constraint_is_reported() {
    let built = json!({"usage": [{"resource": "x", "versionConstraint": "^2.0.0"}]});
    let published = json!({"usage": [{"resource": "x", "versionConstraint": "1.0.0"}]});
    assert_eq!(differences(&built, &published, &[]).len(), 1);
}

#[test]
fn created_is_ignored() {
    let built = json!({"created": "2026-01-01T00:00:00Z", "index": {}});
    let published = json!({"created": "2001-02-03T04:05:06Z", "index": {}});
    assert_eq!(
        differences(&built, &published, &["created", "checksum"]),
        [] as [crate::stdlib_compare::Difference; 0]
    );
}

/// A build always writes a `checksum` map; three of the ten published
/// libraries have none. Adding one is allowed.
#[test]
fn a_checksum_may_be_added() {
    let built = json!({"checksum": {"a.kerml": {"value": "x"}}, "index": {}});
    let published = json!({"index": {}});
    assert_eq!(
        differences(&built, &published, &["created", "checksum"]),
        [] as [crate::stdlib_compare::Difference; 0]
    );
}

/// Added, but never *changed*: where the published file has a map, it
/// must match.
#[test]
fn a_changed_checksum_is_reported() {
    let built = json!({"checksum": {"a.kerml": {"value": "x"}}, "index": {}});
    let published = json!({"checksum": {"a.kerml": {"value": "y"}}, "index": {}});
    assert_eq!(
        differences(&built, &published, &["created", "checksum"]).len(),
        1
    );
}

/// JSON arrays compare in order, which is why the fixture generator
/// emits usages in the published order.
#[test]
fn usage_order_is_significant() {
    let built = json!({"usage": [{"resource": "a"}, {"resource": "b"}]});
    let published = json!({"usage": [{"resource": "b"}, {"resource": "a"}]});
    assert_ne!(
        differences(&built, &published, &[]),
        [] as [crate::stdlib_compare::Difference; 0]
    );
}

#[test]
fn a_missing_key_is_reported() {
    let built = json!({"name": "dep"});
    let published = json!({"name": "dep", "description": "a library"});
    let found = differences(&built, &published, &[]);
    assert_eq!(found.len(), 1);
    assert!(format!("{}", found[0]).contains("<absent>"), "{found:?}");
}

#[test]
fn published_sources_reports_a_missing_file() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    let error = super::published_sources(dir.path(), &["Base.kerml".to_owned()])
        .expect_err("the file is not there");
    assert!(error.contains("Base.kerml"), "{error}");
}

#[test]
fn published_sources_reads_what_is_there() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::write(dir.path().join("Base.kerml"), b"package Base;\n").expect("a source");
    let sources =
        super::published_sources(dir.path(), &["Base.kerml".to_owned()]).expect("the file reads");
    assert_eq!(
        sources,
        BTreeMap::from([("Base.kerml".to_owned(), b"package Base;\n".to_vec())])
    );
}
