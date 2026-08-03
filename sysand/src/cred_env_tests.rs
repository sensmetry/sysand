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

// `validate_env_groups`: the strict validation shared by the eager
// policy build and `auth status`. Groups are built directly so no test
// mutates the process environment.

use crate::cred_env::{CredEnvGroups, validate_env_groups};

fn add(map: &mut std::collections::HashMap<String, String>, stem: &str, value: &str) {
    map.insert(stem.to_string(), value.to_string());
}

#[test]
fn validate_accepts_complete_groups_of_either_or_both_schemes() {
    let mut groups = CredEnvGroups::default();
    add(&mut groups.patterns, "BRR", "https://a.example/**");
    add(&mut groups.bearer_tokens, "BRR", "tok");
    add(&mut groups.patterns, "BSC", "https://b.example/**");
    add(&mut groups.basic_users, "BSC", "user");
    add(&mut groups.basic_passwords, "BSC", "pass");
    add(&mut groups.patterns, "BOTH", "https://c.example/**");
    add(&mut groups.bearer_tokens, "BOTH", "tok");
    add(&mut groups.basic_users, "BOTH", "user");
    add(&mut groups.basic_passwords, "BOTH", "pass");
    assert!(validate_env_groups(&groups).is_ok());
}

#[test]
fn validate_rejects_label_less_names_before_group_checks() {
    let mut groups = CredEnvGroups::default();
    groups.missing_label.push("SYSAND_CRED".to_string());
    // Also scheme-less; the label error must win.
    add(&mut groups.patterns, "A", "https://a.example/**");
    let err = validate_env_groups(&groups).unwrap_err().to_string();
    assert!(err.contains("SYSAND_CRED has no label"), "{err}");
}

#[test]
fn validate_rejects_a_pattern_without_a_scheme() {
    let mut groups = CredEnvGroups::default();
    add(&mut groups.patterns, "A", "https://a.example/**");
    let err = validate_env_groups(&groups).unwrap_err().to_string();
    assert!(
        err.contains("SYSAND_CRED_A has no matching authentication scheme"),
        "{err}"
    );
}

#[test]
fn validate_rejects_half_of_a_basic_pair() {
    let mut groups = CredEnvGroups::default();
    add(&mut groups.patterns, "A", "https://a.example/**");
    add(&mut groups.basic_users, "A", "user");
    let err = validate_env_groups(&groups).unwrap_err().to_string();
    assert!(
        err.contains("both (or neither) of SYSAND_CRED_A_BASIC_USER and SYSAND_CRED_A_BASIC_PASS"),
        "{err}"
    );
}

#[test]
fn validate_rejects_a_secret_without_a_pattern() {
    let mut groups = CredEnvGroups::default();
    add(&mut groups.bearer_tokens, "A", "tok");
    let err = validate_env_groups(&groups).unwrap_err().to_string();
    assert!(
        err.contains("please specify URL pattern SYSAND_CRED_A for credential"),
        "{err}"
    );
}

#[test]
fn validate_reports_the_alphabetically_first_malformed_group() {
    // Deterministic even though the role maps are hash maps.
    let mut groups = CredEnvGroups::default();
    add(&mut groups.patterns, "ZULU", "https://z.example/**");
    add(&mut groups.patterns, "ALFA", "https://a.example/**");
    let err = validate_env_groups(&groups).unwrap_err().to_string();
    assert!(err.contains("SYSAND_CRED_ALFA"), "{err}");
}
