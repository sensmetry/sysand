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
        name: "json_hash_agrees_with_shell".to_string(),
        publisher: None,
        description: None,
        version: "1.2.3".to_string(),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    };

    let meta = InterchangeProjectMetadataRaw {
        index: IndexMap::new(),
        created: "0000-00-00T00:00:00.123456789Z".to_string(),
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
