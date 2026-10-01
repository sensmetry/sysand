// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use super::{ConstraintChange, SetConstraintError, do_set_usage_constraint};

const LIBRARY: &str = "pkg:sysand/mock/library";
/// The project the manifest's directory usage points at.
const LOCAL_LIB: &str = "pkg:sysand/local-pub/local-lib";
const LEGACY: &str = ">=0.10.0, <0.11.0";
const TARGET: &str = ">=0.11.0, <0.12.0";

/// A manifest as sysand writes it, carrying an unknown top-level key, an
/// unknown key inside the target usage, non-canonical key order, and a
/// directory usage that must never match a resource lookup.
const MANIFEST: &str = r#"{
  "version": "1.2.3",
  "name": "fidelity",
  "x-unknown-top-level": {
    "kept": true
  },
  "publisher": "acme",
  "usage": [
    {
      "dir": "../local-lib",
      "publisher": "local-pub",
      "name": "local-lib"
    },
    {
      "x-unknown-in-usage": null,
      "resource": "pkg:sysand/mock/library",
      "versionConstraint": ">=0.10.0, <0.11.0"
    }
  ]
}
"#;

fn doc(text: &str) -> serde_json::Value {
    serde_json::from_str(text).expect("test manifest must parse")
}

fn render(doc: &serde_json::Value) -> String {
    let mut text = serde_json::to_string_pretty(doc).expect("render");
    text.push('\n');
    text
}

fn one_line_diff<'a>(before: &'a str, after: &'a str) -> (&'a str, &'a str) {
    let before: Vec<_> = before.lines().collect();
    let after: Vec<_> = after.lines().collect();
    assert_eq!(before.len(), after.len(), "line count must not change");
    let differing: Vec<_> = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| b != a)
        .map(|(b, a)| (*b, *a))
        .collect();
    assert_eq!(
        differing.len(),
        1,
        "exactly one line must differ: {differing:?}"
    );
    differing[0]
}

#[test]
fn replace_touches_exactly_one_line() {
    let mut d = doc(MANIFEST);

    let change = do_set_usage_constraint(&mut d, LIBRARY, TARGET).unwrap();

    assert_eq!(
        change,
        ConstraintChange::Replaced {
            old: Some(LEGACY.to_owned()),
            new: TARGET.to_owned(),
        }
    );
    let after = render(&d);
    let (old_line, new_line) = one_line_diff(MANIFEST, &after);
    assert_eq!(
        old_line,
        r#"      "versionConstraint": ">=0.10.0, <0.11.0""#
    );
    assert_eq!(
        new_line,
        r#"      "versionConstraint": ">=0.11.0, <0.12.0""#
    );
}

#[test]
fn the_shorthand_is_not_expanded() {
    // `publisher/name` is not an IRI; it matches nothing, not even the
    // `pkg:sysand` usage it abbreviates.
    let mut d = doc(MANIFEST);

    let change = do_set_usage_constraint(&mut d, "mock/library", TARGET).unwrap();

    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn set_when_absent_appends_the_key() {
    let mut d = doc(&MANIFEST.replace(",\n      \"versionConstraint\": \">=0.10.0, <0.11.0\"", ""));

    let change = do_set_usage_constraint(&mut d, LIBRARY, "^0.11").unwrap();

    assert_eq!(
        change,
        ConstraintChange::Replaced {
            old: None,
            new: "^0.11".to_owned(),
        }
    );
    assert_eq!(render(&d), MANIFEST.replace(LEGACY, "^0.11"));
}

#[test]
fn null_constraint_counts_as_absent_and_keeps_its_position() {
    let mut d = doc(&MANIFEST.replace(&format!("\"{LEGACY}\""), "null"));

    let change = do_set_usage_constraint(&mut d, LIBRARY, TARGET).unwrap();

    assert_eq!(
        change,
        ConstraintChange::Replaced {
            old: None,
            new: TARGET.to_owned(),
        }
    );
    assert_eq!(render(&d), MANIFEST.replace(LEGACY, TARGET));
}

#[test]
fn unchanged_leaves_the_document_alone() {
    let mut d = doc(MANIFEST);

    let change = do_set_usage_constraint(&mut d, LIBRARY, LEGACY).unwrap();

    assert_eq!(
        change,
        ConstraintChange::Unchanged {
            constraint: LEGACY.to_owned(),
        }
    );
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn not_found_without_usage_key_and_without_match() {
    let mut d = doc(r#"{"name": "n", "version": "1.0.0"}"#);
    let change = do_set_usage_constraint(&mut d, "pkg:sysand/acme/absent", "1").unwrap();
    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(d, doc(r#"{"name": "n", "version": "1.0.0"}"#));

    let mut d = doc(MANIFEST);
    let change = do_set_usage_constraint(&mut d, "pkg:sysand/acme/absent", "1").unwrap();
    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn directory_usage_is_refused_rather_than_reported_missing() {
    // The directory usage is the same project the PURL names, so
    // "not declared" would be false; it just cannot carry a constraint.
    let mut d = doc(MANIFEST);
    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, "1").unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::UsageCannotHoldConstraint {
            identifier,
            kind: "a directory",
        } if identifier == LOCAL_LIB
    );
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn directory_usage_matches_through_the_normalized_identifier() {
    // `Directory` stores `publisher`/`name` unnormalized, while the
    // identifier lowercases them and turns spaces into hyphens. Comparing
    // the raw strings would miss this.
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({
        "dir": "../local-lib",
        "publisher": "Local Pub",
        "name": "Local-Lib",
    });
    let before = d.clone();

    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, "1").unwrap_err();

    assert_matches!(err, SetConstraintError::UsageCannotHoldConstraint { .. });
    assert_eq!(d, before);
}

#[test]
fn kpar_path_usage_is_refused_by_its_identifier() {
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({
        "kparPath": "../local-lib.kpar",
        "publisher": "local-pub",
        "name": "local-lib",
    });

    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, "1").unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::UsageCannotHoldConstraint {
            kind: "a KPAR path",
            ..
        }
    );
}

#[test]
fn a_resource_and_a_typed_usage_of_one_project_are_ambiguous() {
    // Declared twice, once per kind. Editing either would be a guess.
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({
        "dir": "../mock-library",
        "publisher": "mock",
        "name": "library",
    });
    let before = d.clone();

    let err = do_set_usage_constraint(&mut d, LIBRARY, TARGET).unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::Ambiguous {
            resource,
            count: 2,
        } if resource == LIBRARY
    );
    assert_eq!(d, before);
}

#[test]
fn an_unrecognized_usage_shape_is_left_alone() {
    // Neither a resource nor a kind sysand knows: not matched, not refused.
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({ "x-future-kind": "somewhere" });

    let change = do_set_usage_constraint(&mut d, LOCAL_LIB, "1").unwrap();

    assert_eq!(change, ConstraintChange::NotFound);
}

#[test]
fn ambiguous_is_refused_before_editing() {
    // Turn the directory usage into a second resource usage of the library.
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({ "resource": LIBRARY });
    let before = d.clone();

    let err = do_set_usage_constraint(&mut d, LIBRARY, TARGET).unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::Ambiguous {
            resource,
            count: 2,
        } if resource == LIBRARY
    );
    assert_eq!(d, before);
}

#[test]
fn invalid_constraint_is_rejected_before_editing() {
    let mut d = doc(MANIFEST);

    let err = do_set_usage_constraint(&mut d, LIBRARY, "nonsense").unwrap_err();

    assert_matches!(err, SetConstraintError::InvalidConstraint(c, _) if c == "nonsense");
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn malformed_documents_are_rejected() {
    let mut d = doc("[]");
    assert_matches!(
        do_set_usage_constraint(&mut d, LIBRARY, "1").unwrap_err(),
        SetConstraintError::NotAnObject
    );

    let mut d = doc(r#"{"usage": {"resource": "x"}}"#);
    assert_matches!(
        do_set_usage_constraint(&mut d, LIBRARY, "1").unwrap_err(),
        SetConstraintError::UsageNotAnArray
    );
}

#[cfg(feature = "filesystem")]
mod filesystem {
    use camino_tempfile::tempdir;
    use std::assert_matches;

    use super::{LEGACY, LIBRARY, MANIFEST, TARGET, one_line_diff};
    use crate::{
        project::local_src::{EditInfoError, LocalSrcProject},
        usage::{ConstraintChange, SetConstraintError, do_set_usage_constraint_local},
    };

    #[test]
    fn sysand_formatted_file_changes_in_one_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".project.json");
        std::fs::write(&path, MANIFEST).unwrap();
        let mut project = LocalSrcProject::new_access(dir.path(), None);

        let change = do_set_usage_constraint_local(&mut project, LIBRARY, TARGET).unwrap();

        assert_eq!(
            change,
            ConstraintChange::Replaced {
                old: Some(LEGACY.to_owned()),
                new: TARGET.to_owned(),
            }
        );
        let after = std::fs::read_to_string(&path).unwrap();
        one_line_diff(MANIFEST, &after);
    }

    #[test]
    fn unchanged_does_not_touch_the_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".project.json");
        std::fs::write(&path, MANIFEST).unwrap();
        let old_mtime = filetime_of(&path);
        let mut project = LocalSrcProject::new_access(dir.path(), None);

        let change = do_set_usage_constraint_local(&mut project, LIBRARY, LEGACY).unwrap();

        assert_matches!(change, ConstraintChange::Unchanged { .. });
        assert_eq!(filetime_of(&path), old_mtime);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANIFEST);
    }

    #[test]
    fn hand_formatted_file_gets_sysand_whitespace_but_keeps_keys_and_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".project.json");
        std::fs::write(
            &path,
            r#"{"name":"hand","version":"1.0.0","usage":[{"resource":"a:b","versionConstraint":"1.0.0"}],"z":1}"#,
        )
        .unwrap();
        let mut project = LocalSrcProject::new_access(dir.path(), None);

        do_set_usage_constraint_local(&mut project, "a:b", "2.0.0").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{
  "name": "hand",
  "version": "1.0.0",
  "usage": [
    {
      "resource": "a:b",
      "versionConstraint": "2.0.0"
    }
  ],
  "z": 1
}
"#
        );
    }

    #[test]
    fn edit_errors_leave_the_file_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".project.json");
        std::fs::write(&path, MANIFEST).unwrap();
        let mut project = LocalSrcProject::new_access(dir.path(), None);

        let err = do_set_usage_constraint_local(&mut project, LIBRARY, "nonsense").unwrap_err();

        assert_matches!(
            err,
            EditInfoError::Edit(SetConstraintError::InvalidConstraint(..))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANIFEST);
    }

    #[test]
    fn missing_manifest_is_a_project_error() {
        let dir = tempdir().unwrap();
        let mut project = LocalSrcProject::new_access(dir.path(), None);

        let err = do_set_usage_constraint_local(&mut project, LIBRARY, "1").unwrap_err();

        assert_matches!(err, EditInfoError::Project(_));
    }

    fn filetime_of(path: &camino::Utf8Path) -> std::time::SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }
}

mod index_usage {
    use super::*;
    use crate::commands::usage::do_set_index_usage_constraint;

    /// Keys in a non-canonical order, to check they stay so
    const INDEX_MANIFEST: &str = r#"{
  "name": "fidelity",
  "version": "1.2.3",
  "usage": [
    {
      "versionConstraint": "^1",
      "name": "My Lib",
      "publisher": "Acme Labs"
    },
    {
      "resource": "pkg:sysand/mock/library"
    }
  ]
}
"#;

    #[test]
    fn edits_the_constraint_in_place() {
        let mut d = doc(INDEX_MANIFEST);

        let change = do_set_index_usage_constraint(&mut d, "Acme Labs", "My Lib", "^2").unwrap();

        assert_eq!(
            change,
            ConstraintChange::Replaced {
                old: Some("^1".to_owned()),
                new: "^2".to_owned(),
            }
        );
        assert_eq!(
            one_line_diff(INDEX_MANIFEST, &render(&d)),
            (
                r#"      "versionConstraint": "^1","#,
                r#"      "versionConstraint": "^2","#
            )
        );
    }

    #[test]
    fn not_found() {
        let mut d = doc(INDEX_MANIFEST);

        let change = do_set_index_usage_constraint(&mut d, "Acme Labs", "Other", "^2").unwrap();

        assert_eq!(change, ConstraintChange::NotFound);
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn different_spelling_is_refused() {
        let mut d = doc(INDEX_MANIFEST);

        let err = do_set_index_usage_constraint(&mut d, "acme labs", "my lib", "^2").unwrap_err();

        assert_matches!(
            err,
            SetConstraintError::IndexUsageSpelledDifferently { existing, .. }
                if existing == "Acme Labs/My Lib"
        );
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn legacy_purl_is_not_matched_by_publisher_and_name() {
        let mut d = doc(INDEX_MANIFEST);

        let err = do_set_index_usage_constraint(&mut d, "mock", "library", "^2").unwrap_err();

        assert_matches!(err, SetConstraintError::NotAnIndexUsage { identifier } if identifier == LIBRARY);
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn purl_of_an_index_usage_points_at_publisher_and_name() {
        let mut d = doc(INDEX_MANIFEST);

        let err = do_set_usage_constraint(&mut d, "pkg:sysand/acme-labs/my-lib", "^2").unwrap_err();

        assert_matches!(
            err,
            SetConstraintError::IndexUsageMatchedByIdentifier { publisher, name, .. }
                if publisher == "Acme Labs" && name == "My Lib"
        );
    }
}
