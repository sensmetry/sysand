// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{blob_full_message, cred_env_var_stem};
use chrono::{Duration, Utc};

#[test]
fn cred_env_var_stem_uses_the_uppercased_host() {
    assert_eq!(
        cred_env_var_stem("https://sysand.example/idx/"),
        "SYSAND_EXAMPLE"
    );
}

#[test]
fn cred_env_var_stem_includes_a_non_default_port() {
    // Two indexes on different ports of one host must not suggest the
    // same variable names.
    assert_eq!(
        cred_env_var_stem("https://sysand.example:8443/"),
        "SYSAND_EXAMPLE_8443"
    );
    assert_ne!(
        cred_env_var_stem("https://sysand.example:8443/"),
        cred_env_var_stem("https://sysand.example:9000/")
    );
}

#[test]
fn cred_env_var_stem_omits_a_scheme_default_port() {
    assert_eq!(
        cred_env_var_stem("https://sysand.example:443/"),
        "SYSAND_EXAMPLE"
    );
    assert_eq!(
        cred_env_var_stem("http://sysand.example:80/"),
        "SYSAND_EXAMPLE"
    );
}

#[test]
fn cred_env_var_stem_never_produces_a_reserved_suffix() {
    // Host characters map to alphanumerics or `_`, and a port adds
    // only digits, so no stem can end in a reserved secret suffix.
    for key in [
        "https://basic.user/",
        "https://x.example:1234/",
        "https://bearer-token.example/",
    ] {
        let stem = cred_env_var_stem(key);
        for suffix in ["_BASIC_USER", "_BASIC_PASS", "_BEARER_TOKEN"] {
            assert!(!stem.ends_with(suffix), "stem {stem} ends in {suffix}");
        }
    }
}

#[test]
fn blob_full_first_login_does_not_suggest_removing_a_login() {
    let message = blob_full_message(&[], Utc::now());
    assert!(message.contains("use a smaller token"), "{message}");
    assert!(!message.contains("logout"), "{message}");
}

#[test]
fn blob_full_names_the_status_and_logout_commands() {
    let now = Utc::now();
    let stored = [("https://a.example/", None), ("https://b.example/", None)];
    let message = blob_full_message(&stored, now);
    assert!(message.contains("sysand auth status"), "{message}");
    assert!(message.contains("sysand auth logout <index>"), "{message}");
}

#[test]
fn blob_full_lists_expired_logins_as_drop_candidates() {
    let now = Utc::now();
    let stored = [
        ("https://live.example/", Some(now + Duration::days(30))),
        ("https://dead.example/", Some(now - Duration::days(1))),
        ("https://unknown.example/", None),
    ];
    let message = blob_full_message(&stored, now);
    assert!(
        message.contains("sysand auth logout https://dead.example/"),
        "{message}"
    );
    assert!(
        !message.contains("logout https://live.example/"),
        "{message}"
    );
    assert!(
        !message.contains("logout https://unknown.example/"),
        "{message}"
    );
}
