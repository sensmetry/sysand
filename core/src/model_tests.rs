// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use indexmap::IndexMap;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    utils::lowercase_hex,
};

#[test]
fn str_hash_agrees_with_shell() {
    // cat <(echo -n "foobar") <(echo -n "bazbum") | sha256sum | cut -f 1 -d ' '
    // ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^_ just a fancy way to write echo -n "foobarbazbum"
    //                                              as if concatenated from two separate files
    assert_eq!(
        lowercase_hex(super::project_hash_str("foobar", "bazbum")),
        "e6e2e042d1d461877c7e79cc890af5de00f603739c17486dc1464acfc0f77797"
    );
}

#[test]
fn json_hash_agrees_with_shell() {
    let info = InterchangeProjectInfoRaw {
        name: "json_hash_agrees_with_shell".to_owned(),
        publisher: None,
        description: None,
        version: "1.2.3".to_owned(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };

    let meta = InterchangeProjectMetadataRaw {
        index: IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_owned(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    };

    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        r#"{"name":"json_hash_agrees_with_shell","version":"1.2.3"}"#
    );
    assert_eq!(
        serde_json::to_string(&meta).unwrap(),
        r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#
    );

    // cat <(echo -n '{"name":"json_hash_agrees_with_shell","version":"1.2.3"}') <(echo -n '{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}') | sha256sum | cut -f 1 -d ' '
    assert_eq!(
        lowercase_hex(super::project_hash_raw(&info, &meta)),
        "3b08c7119d89c406de6bdfbed29566077209d295736264229ad5d2e33991b3b4"
    );
}

/// `.project.json` shipped by a newer sysand, declaring a usage kind this
/// build has never heard of.
const MANIFEST_WITH_A_FUTURE_USAGE: &str = r#"{
  "name": "main",
  "publisher": "acme",
  "version": "1.2.3",
  "usage": [
    { "resource": "pkg:sysand/acme/lib", "versionConstraint": "^1" },
    { "registry": "https://example.com/i", "publisher": "acme", "name": "future" }
  ]
}"#;

#[test]
fn a_future_usage_kind_parses_instead_of_failing_the_whole_manifest() {
    let info: InterchangeProjectInfoRaw =
        serde_json::from_str(MANIFEST_WITH_A_FUTURE_USAGE).unwrap();

    // The manifest is readable, and the usage sysand *does* understand is
    // still understood.
    assert_eq!(info.name, "main");
    assert_eq!(info.usage.len(), 2);
    assert!(matches!(
        info.usage[0],
        super::InterchangeProjectUsageRaw::Resource { .. }
    ));
    let super::InterchangeProjectUsageRaw::Unknown(unknown) = &info.usage[1] else {
        panic!(
            "expected the second usage to be unrecognized: {:?}",
            info.usage[1]
        );
    };
    assert_eq!(unknown.quoted_keys(), "`registry`, `publisher`, `name`");
}

#[test]
fn a_future_usage_kind_survives_a_round_trip() {
    let info: InterchangeProjectInfoRaw =
        serde_json::from_str(MANIFEST_WITH_A_FUTURE_USAGE).unwrap();

    let written = serde_json::to_value(&info).unwrap();

    // Every key, value and key order of the entry sysand cannot interpret is
    // written back exactly as it was read: rewriting a manifest never drops a
    // dependency a newer sysand declared.
    assert_eq!(
        written["usage"][1],
        serde_json::json!({
            "registry": "https://example.com/i",
            "publisher": "acme",
            "name": "future"
        })
    );
}

#[test]
fn a_future_usage_kind_is_refused_when_it_has_to_be_interpreted() {
    let info: InterchangeProjectInfoRaw =
        serde_json::from_str(MANIFEST_WITH_A_FUTURE_USAGE).unwrap();

    let error = info.usage[1].validate().unwrap_err();

    // Tolerated at the parse boundary, refused the moment a caller would act
    // on it as a dependency -- so it can never be silently skipped.
    assert!(
        matches!(
            error,
            super::InterchangeProjectValidationError::UnknownUsageKind { .. }
        ),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(message.contains("`registry`"), "{message}");
    assert!(message.contains("newer sysand"), "{message}");
}

#[test]
fn a_usage_entry_that_is_not_an_object_still_fails_to_parse() {
    // The catch-all matches a JSON *object*. A usage that is not one is
    // malformed in a way no future kind can explain, so it stays an error.
    let error = serde_json::from_str::<InterchangeProjectInfoRaw>(
        r#"{ "name": "main", "version": "1.2.3", "usage": ["not-an-object"] }"#,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("did not match any variant of untagged enum"),
        "{error}"
    );
}
