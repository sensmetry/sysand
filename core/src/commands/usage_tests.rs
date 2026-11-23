// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use super::{ConstraintChange, SetConstraintError, do_set_usage_constraint};

const LIBRARY: &str = "pkg:sysand/mock/library";
/// The project the manifest's directory usage points at.
const LOCAL_LIB: &str = "pkg:sysand/local-pub/local-lib";
const LEGACY: &str = ">=0.10.0, <0.11.0";
const TARGET: &str = ">=0.11.0, <0.12.0";

fn req(constraint: &str) -> semver::VersionReq {
    semver::VersionReq::parse(constraint).expect("a version requirement")
}

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

    let change = do_set_usage_constraint(&mut d, LIBRARY, &req(TARGET)).unwrap();

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

    let change = do_set_usage_constraint(&mut d, "mock/library", &req(TARGET)).unwrap();

    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn set_when_absent_appends_the_key() {
    let mut d = doc(&MANIFEST.replace(
        r#",
      "versionConstraint": ">=0.10.0, <0.11.0""#,
        "",
    ));

    let change = do_set_usage_constraint(&mut d, LIBRARY, &req("^0.11")).unwrap();

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

    let change = do_set_usage_constraint(&mut d, LIBRARY, &req(TARGET)).unwrap();

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

    let change = do_set_usage_constraint(&mut d, LIBRARY, &req(LEGACY)).unwrap();

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
    let change = do_set_usage_constraint(&mut d, "pkg:sysand/acme/absent", &req("1")).unwrap();
    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(d, doc(r#"{"name": "n", "version": "1.0.0"}"#));

    let mut d = doc(MANIFEST);
    let change = do_set_usage_constraint(&mut d, "pkg:sysand/acme/absent", &req("1")).unwrap();
    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(render(&d), MANIFEST);
}

#[test]
fn directory_usage_is_refused_rather_than_reported_missing() {
    // The directory usage is the same project the PURL names, so
    // "not declared" would be false; it just cannot carry a constraint.
    let mut d = doc(MANIFEST);
    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, &req("1")).unwrap_err();

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

    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, &req("1")).unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::UsageCannotHoldConstraint {
            identifier: _,
            kind: _
        }
    );
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

    let err = do_set_usage_constraint(&mut d, LOCAL_LIB, &req("1")).unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::UsageCannotHoldConstraint {
            kind: "a KPAR path",
            identifier: _
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

    let err = do_set_usage_constraint(&mut d, LIBRARY, &req(TARGET)).unwrap_err();

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
fn an_unrecognized_usage_shape_is_refused() {
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({ "x-future-kind": "somewhere" });
    let before = d.clone();

    let err = do_set_usage_constraint(&mut d, LIBRARY, &req("1")).unwrap_err();

    assert_matches!(
        err,
        SetConstraintError::InvalidUsage {
            index: 0,
            source: _
        }
    );
    assert_eq!(d, before);
}

#[test]
fn ambiguous_is_refused_before_editing() {
    // Turn the directory usage into a second resource usage of the library.
    let mut d = doc(MANIFEST);
    d["usage"][0] = serde_json::json!({ "resource": LIBRARY });
    let before = d.clone();

    let err = do_set_usage_constraint(&mut d, LIBRARY, &req(TARGET)).unwrap_err();

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
fn malformed_documents_are_rejected() {
    let mut d = doc("[]");
    assert_matches!(
        do_set_usage_constraint(&mut d, LIBRARY, &req("1")).unwrap_err(),
        SetConstraintError::NotAnObject
    );

    let mut d = doc(r#"{"usage": {"resource": "x"}}"#);
    assert_matches!(
        do_set_usage_constraint(&mut d, LIBRARY, &req("1")).unwrap_err(),
        SetConstraintError::UsageNotAnArray
    );
}

#[cfg(feature = "filesystem")]
mod filesystem {
    use camino_tempfile::{Utf8TempDir, tempdir};
    use std::assert_matches;

    use super::{LEGACY, LIBRARY, MANIFEST, TARGET, one_line_diff};
    use crate::{
        context::{EditError, ManifestStep, ProjectContext},
        model::InterchangeProjectMetadataRaw,
        project::{
            ProjectRead as _,
            local_src::{LocalSrcError, LocalSrcProject},
        },
        usage::{
            ConstraintChange, SetConstraintError, SetUsageConstraint, SetUsageConstraintError,
            UsageTarget,
        },
    };

    /// A directory holding `info` as `.project.json` and a minimal
    /// `.meta.json`.
    fn json_project(info: &str) -> (Utf8TempDir, camino::Utf8PathBuf) {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".project.json");
        std::fs::write(&path, info).unwrap();
        let meta = InterchangeProjectMetadataRaw::minimal("1970-01-01T00:00:00Z".to_owned());
        std::fs::write(
            dir.path().join(".meta.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .unwrap();
        (dir, path)
    }

    fn context(dir: &Utf8TempDir) -> ProjectContext {
        let project = LocalSrcProject::new_access(dir.path(), None);
        let definition = project.get_project().unwrap();
        ProjectContext::new(None, Some(project), definition)
    }

    fn set(
        ctx: &mut ProjectContext,
        resource: &str,
        constraint: &str,
    ) -> Result<ConstraintChange, EditError<ManifestStep<SetUsageConstraintError>>> {
        ctx.edit(SetUsageConstraint {
            target: UsageTarget::Resource(resource.to_owned()),
            constraint: super::req(constraint),
        })
    }

    #[test]
    fn a_directory_with_both_manifests_is_not_a_project() {
        let (dir, path) = json_project(MANIFEST);
        std::fs::write(dir.path().join("model.toml"), "").unwrap();

        let err = LocalSrcProject::new_access(dir.path(), None)
            .get_project()
            .unwrap_err();

        assert_matches!(
            err,
            LocalSrcError::Format {
                problem: crate::project::FormatError::Ambiguous { json: _ },
                dir: _
            }
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANIFEST);
    }

    #[test]
    fn sysand_formatted_file_changes_in_one_line() {
        let (dir, path) = json_project(MANIFEST);

        let change = set(&mut context(&dir), LIBRARY, TARGET).unwrap();

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
        let (dir, path) = json_project(MANIFEST);
        let old_mtime = filetime_of(&path);

        let change = set(&mut context(&dir), LIBRARY, LEGACY).unwrap();

        assert_matches!(change, ConstraintChange::Unchanged { constraint: _ });
        assert_eq!(filetime_of(&path), old_mtime);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANIFEST);
    }

    #[test]
    fn hand_formatted_file_gets_sysand_whitespace_but_keeps_keys_and_order() {
        let (dir, path) = json_project(
            r#"{"name":"hand","version":"1.0.0","usage":[{"resource":"a:b","versionConstraint":"1.0.0"}],"z":1}"#,
        );

        set(&mut context(&dir), "a:b", "2.0.0").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{
  "name": "hand",
  "version": "1.0.0",
  "usage": [
    {
      "resource": "a:b",
      "versionConstraint": "^2.0.0"
    }
  ],
  "z": 1
}
"#
        );
    }

    #[test]
    fn edit_errors_leave_the_file_untouched() {
        let (dir, path) = json_project(MANIFEST);

        let err = set(&mut context(&dir), super::LOCAL_LIB, "^1").unwrap_err();

        assert_matches!(
            err,
            EditError::Edit(ManifestStep::Edit(SetUsageConstraintError::Edit(
                SetConstraintError::UsageCannotHoldConstraint {
                    identifier: _,
                    kind: _
                }
            )))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), MANIFEST);
    }

    #[test]
    fn a_project_without_a_manifest_has_nothing_to_edit() {
        let dir = tempdir().unwrap();
        let project = LocalSrcProject::new_access(dir.path(), None);
        let mut ctx = ProjectContext::new(None, Some(project), None);

        let err = set(&mut ctx, LIBRARY, "1").unwrap_err();

        assert_matches!(err, EditError::NoManifest);
    }

    fn filetime_of(path: &camino::Utf8Path) -> std::time::SystemTime {
        std::fs::metadata(path).unwrap().modified().unwrap()
    }
}

mod index_usage {
    use super::*;
    use crate::commands::usage::do_set_index_usage_constraint;

    fn index_publisher(publisher: &str) -> IndexPublisher {
        IndexPublisher::parse(publisher.to_owned()).unwrap()
    }

    fn index_name(name: &str) -> IndexName {
        IndexName::parse(name.to_owned()).unwrap()
    }

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

        let change = do_set_index_usage_constraint(
            &mut d,
            &index_publisher("Acme Labs"),
            &index_name("My Lib"),
            &req("^2"),
        )
        .unwrap();

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

        let change = do_set_index_usage_constraint(
            &mut d,
            &index_publisher("Acme Labs"),
            &index_name("Other"),
            &req("^2"),
        )
        .unwrap();

        assert_eq!(change, ConstraintChange::NotFound);
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn an_unrecognized_usage_shape_is_refused() {
        let mut d = doc(INDEX_MANIFEST);
        d["usage"][1] = serde_json::json!({ "x-future-kind": "somewhere" });
        let before = d.clone();

        let err = do_set_index_usage_constraint(
            &mut d,
            &index_publisher("Acme Labs"),
            &index_name("My Lib"),
            &req("^2"),
        )
        .unwrap_err();

        assert_matches!(
            err,
            SetConstraintError::InvalidUsage {
                index: 1,
                source: _
            }
        );
        assert_eq!(d, before);
    }

    #[test]
    fn different_spelling_is_refused() {
        let mut d = doc(INDEX_MANIFEST);

        let err = do_set_index_usage_constraint(
            &mut d,
            &index_publisher("acme labs"),
            &index_name("my lib"),
            &req("^2"),
        )
        .unwrap_err();

        assert_matches!(
            err,
            SetConstraintError::IndexUsageSpelledDifferently { existing, requested: _ }
                if existing == "Acme Labs/My Lib"
        );
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn legacy_purl_is_not_matched_by_publisher_and_name() {
        let mut d = doc(INDEX_MANIFEST);

        let err = do_set_index_usage_constraint(
            &mut d,
            &index_publisher("mock"),
            &index_name("library"),
            &req("^2"),
        )
        .unwrap_err();

        assert_matches!(err, SetConstraintError::NotAnIndexUsage { identifier } if identifier == LIBRARY);
        assert_eq!(render(&d), INDEX_MANIFEST);
    }

    #[test]
    fn purl_of_an_index_usage_points_at_publisher_and_name() {
        let mut d = doc(INDEX_MANIFEST);

        let err =
            do_set_usage_constraint(&mut d, "pkg:sysand/acme-labs/my-lib", &req("^2")).unwrap_err();

        assert_matches!(
            err,
            SetConstraintError::IndexUsageMatchedByIdentifier { publisher, name, identifier: _ }
                if publisher == "Acme Labs" && name == "My Lib"
        );
    }
}

// ---------------------------------------------------------------------
// The `model.toml` arm.

use super::{do_set_index_usage_constraint_toml, do_set_usage_constraint_toml};
use crate::model::{IndexName, IndexPublisher};

const TOML: &str = r#"manifest-version = "1.0"

[project]
publisher = "acme"
name = "app"
version = "1.0.0"
sources = []
metamodel = "https://www.omg.org/spec/SysML/20250201"

[[inline-usages]]
publisher = "acme"
name = "lib"
# pinned for the demo
version = "^1"

[[inline-usages]]
publisher = "acme"
name = "other"
version = "^3"

[[inline-usages]]
publisher = "acme"
name = "local"
dir = "../local"
"#;

fn toml_doc(text: &str) -> toml_edit::DocumentMut {
    text.parse().expect("test manifest must parse")
}

/// The identity form, as the bindings reach it for `publisher`/`name`.
fn by_identity(
    doc: &mut toml_edit::DocumentMut,
    publisher: &str,
    name: &str,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    do_set_index_usage_constraint_toml(
        doc,
        &IndexPublisher::parse(publisher.to_owned()).unwrap(),
        &IndexName::parse(name.to_owned()).unwrap(),
        &req(constraint),
    )
}

#[test]
fn toml_replace_touches_exactly_one_line() {
    let mut doc = toml_doc(TOML);
    let change =
        do_set_usage_constraint_toml(&mut doc, "pkg:sysand/acme/lib", &req("^2")).expect("ok");
    assert_eq!(
        change,
        ConstraintChange::Replaced {
            old: Some("^1".to_owned()),
            new: "^2".to_owned(),
        }
    );
    let rendered = doc.to_string();
    let (before, after) = one_line_diff(TOML, &rendered);
    assert_eq!(before, "version = \"^1\"");
    assert_eq!(after, "version = \"^2\"");
    assert!(doc.to_string().contains("# pinned for the demo"));
}

#[test]
fn toml_identity_matches_the_declaration() {
    let mut doc = toml_doc(TOML);
    let change = by_identity(&mut doc, "acme", "other", ">=3, <5").expect("ok");
    assert_matches!(change, ConstraintChange::Replaced { old: _, new: _ });
    assert!(doc.to_string().contains("version = \">=3, <5\""));
}

#[test]
fn toml_unchanged_and_not_found_leave_the_document_alone() {
    let mut doc = toml_doc(TOML);
    let change = by_identity(&mut doc, "acme", "lib", "^1").expect("ok");
    assert_eq!(
        change,
        ConstraintChange::Unchanged {
            constraint: "^1".to_owned()
        }
    );
    let change = by_identity(&mut doc, "acme", "missing", "^1").expect("ok");
    assert_eq!(change, ConstraintChange::NotFound);
    assert_eq!(doc.to_string(), TOML);
}

/// An index usage requires a `version`, so the widest constraint, `*`, is
/// written in place of the old one.
#[test]
fn a_toml_index_usage_keeps_a_constraint() {
    let mut doc = toml_doc(TOML);
    let change = by_identity(&mut doc, "acme", "lib", "*").expect("ok");
    assert_matches!(change, ConstraintChange::Replaced { old: _, new: _ });
    let rendered = doc.to_string();
    let (before, after) = one_line_diff(TOML, &rendered);
    assert_eq!(before, "version = \"^1\"");
    assert_eq!(after, "version = \"*\"");
}

/// A `dir` usage has no constraint to set.
#[test]
fn a_dir_usage_has_no_constraint_to_set() {
    let mut doc = toml_doc(TOML);
    let error = by_identity(&mut doc, "acme", "local", "^1").expect_err("refused");
    assert_eq!(
        error.to_string(),
        "`acme/local` is not an index usage and has no version"
    );
    assert_matches!(error, SetConstraintError::TomlUsageHasNoConstraint(_));
    assert_eq!(doc.to_string(), TOML);
}

/// An entry with neither a source nor a `version` is invalid, and is
/// refused like a `dir` usage.
#[test]
fn a_usage_with_no_source_and_no_version_is_refused() {
    let text = TOML.replace("dir = \"../local\"\n", "");
    let mut doc = toml_doc(&text);
    let error = by_identity(&mut doc, "acme", "local", "^1").expect_err("refused");
    assert_matches!(error, SetConstraintError::TomlUsageHasNoConstraint(_));
    assert_eq!(doc.to_string(), text);
}

/// A `model.toml` usage is addressed by identity.
#[test]
fn a_toml_usage_is_addressed_by_identity() {
    let mut doc = toml_doc(TOML);
    let error = do_set_usage_constraint_toml(&mut doc, "urn:kpar:lib", &req("^1"))
        .expect_err("not an identity");
    assert_eq!(
        error.to_string(),
        "`urn:kpar:lib` is not a `pkg:sysand` PURL; a `model.toml` usage is named `publisher/name`"
    );
    // A PURL whose parts are not normalized is malformed, as in the json arm.
    let error = do_set_usage_constraint_toml(&mut doc, "pkg:sysand/Acme/Lib", &req("^1"))
        .expect_err("malformed");
    assert_matches!(
        error,
        SetConstraintError::MalformedPurl { iri: _, source: _ }
    );
    assert_eq!(doc.to_string(), TOML);
}

/// A standard-library entry declares no `publisher` and is reached under
/// `omg`; as a `kpar-url` usage it has no constraint, while an
/// index entry published by `OMG` does.
#[test]
fn a_stdlib_usage_is_reached_under_omg() {
    let text = TOML.replace(
        r#"[[inline-usages]]
publisher = "acme"
name = "local"
dir = "../local"
"#,
        r#"[[inline-usages]]
name = "Kernel Semantic Library"
kpar-url = "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"

[[inline-usages]]
publisher = "OMG"
name = "Widgets"
version = "^1"
"#,
    );
    let mut doc = toml_doc(&text);
    let error = by_identity(&mut doc, "omg", "kernel-semantic-library", "^1")
        .expect_err("a kpar-url usage has no constraint");
    assert_matches!(error, SetConstraintError::TomlUsageHasNoConstraint(_));
    let change = by_identity(&mut doc, "omg", "widgets", "^2").expect("ok");
    assert_matches!(change, ConstraintChange::Replaced { old: _, new: _ });
}
