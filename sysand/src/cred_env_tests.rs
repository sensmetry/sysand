// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::cred_env::{CredEnvName, CredEnvRole, classify};

fn grouped(stem: &str, role: CredEnvRole) -> Option<CredEnvName> {
    Some(CredEnvName::Grouped(stem.to_string(), role))
}

#[test]
fn classify_splits_stem_and_role() {
    for (name, expected) in [
        ("SYSAND_CRED_TEST", grouped("TEST", CredEnvRole::Pattern)),
        (
            "SYSAND_CRED_TEST_BASIC_USER",
            grouped("TEST", CredEnvRole::BasicUser),
        ),
        (
            "SYSAND_CRED_TEST_BASIC_PASS",
            grouped("TEST", CredEnvRole::BasicPass),
        ),
        (
            "SYSAND_CRED_TEST_BEARER_TOKEN",
            grouped("TEST", CredEnvRole::BearerToken),
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
        grouped("BEARER_TOKEN_X", CredEnvRole::Pattern)
    );
}

#[test]
fn classify_flags_names_with_no_label() {
    // The bare prefix, a bare role suffix, and an empty stem all leave no
    // label; none of them may masquerade as a pattern or secret variable.
    for name in [
        "SYSAND_CRED",
        "SYSAND_CRED_",
        "SYSAND_CRED_BASIC_USER",
        "SYSAND_CRED_BASIC_PASS",
        "SYSAND_CRED_BEARER_TOKEN",
        "SYSAND_CRED__BASIC_USER",
        "SYSAND_CRED__BASIC_PASS",
        "SYSAND_CRED__BEARER_TOKEN",
    ] {
        assert_eq!(classify(name), Some(CredEnvName::MissingLabel), "{name}");
    }
}
