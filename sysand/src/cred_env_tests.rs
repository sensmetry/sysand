// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::cred_env::{CredEnvRole, classify};

#[test]
fn classify_splits_stem_and_role() {
    for (name, expected) in [
        (
            "SYSAND_CRED_TEST",
            Some(("TEST".to_string(), CredEnvRole::Pattern)),
        ),
        (
            "SYSAND_CRED_TEST_BASIC_USER",
            Some(("TEST".to_string(), CredEnvRole::BasicUser)),
        ),
        (
            "SYSAND_CRED_TEST_BASIC_PASS",
            Some(("TEST".to_string(), CredEnvRole::BasicPass)),
        ),
        (
            "SYSAND_CRED_TEST_BEARER_TOKEN",
            Some(("TEST".to_string(), CredEnvRole::BearerToken)),
        ),
        ("SYSAND_DEFAULT_INDEX", None),
        ("PATH", None),
    ] {
        assert_eq!(classify(name), expected, "name: {name}");
    }
}

#[test]
fn classify_matches_a_suffix_only_at_the_end() {
    // A suffix-looking infix stays part of the pattern stem.
    assert_eq!(
        classify("SYSAND_CRED_BEARER_TOKEN_X"),
        Some(("BEARER_TOKEN_X".to_string(), CredEnvRole::Pattern))
    );
}
