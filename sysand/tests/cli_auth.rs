// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI tests for `sysand auth status` and `sysand auth logout`.
//!
//! These tests exercise argument parsing, the default-index resolution
//! chain, and status rendering of `SYSAND_CRED_*` entries. Logout tests
//! deliberately fail before any credential store access (parse errors,
//! non-HTTP(S) targets, ambiguous defaults), so no test requires, or can
//! ever mutate, a real OS keyring: store behavior itself is covered by the
//! core tests over `InMemoryCredentialStore`.

use std::fs;

use assert_cmd::prelude::*;
use indexmap::IndexMap;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const NOT_HTTP_MESSAGE: &str = "not an HTTP(S) index; nothing to authenticate to";
const AMBIGUOUS_MESSAGE: &str = "pass an explicit index URL";

fn no_env() -> IndexMap<String, String> {
    IndexMap::new()
}

fn default_index_env(value: &str) -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    env.insert("SYSAND_DEFAULT_INDEX".to_string(), value.to_string());
    env
}

#[test]
fn auth_requires_a_subcommand() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand(["auth"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
    Ok(())
}

#[test]
fn auth_logout_rejects_an_unparseable_url() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand(["auth", "logout", "not a url"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("invalid index URL"));
    Ok(())
}

#[test]
fn auth_logout_rejects_a_non_http_index() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand(["auth", "logout", "file:///srv/index"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    Ok(())
}

#[test]
fn auth_logout_with_an_explicit_url_skips_default_index_resolution() -> TestResult {
    // Two default-index URLs would be ambiguous; the explicit positional
    // URL must win without consulting the chain at all.
    let (_temp_dir, _cwd, out) = run_sysand_with(
        ["auth", "logout", "file:///srv/index"],
        None,
        &default_index_env("https://a.example,https://b.example"),
    )?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains("file:///srv/index"))
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE))
        .stderr(predicate::str::contains(AMBIGUOUS_MESSAGE).not());
    Ok(())
}

#[test]
fn bare_auth_logout_resolves_and_echoes_the_env_default_index() -> TestResult {
    // A file:// default index exercises the resolution chain and the echo
    // while failing before any credential store access.
    let (_temp_dir, _cwd, out) = run_sysand_with(
        ["auth", "logout"],
        None,
        &default_index_env("file:///srv/index"),
    )?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains(
            "Logging out from index `file:///srv/index`",
        ))
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    Ok(())
}

#[test]
fn bare_auth_logout_with_multiple_env_defaults_asks_for_an_explicit_url() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand_with(
        ["auth", "logout"],
        None,
        &default_index_env("https://a.example,https://b.example"),
    )?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("more than one default index"))
        .stderr(predicate::str::contains(AMBIGUOUS_MESSAGE));
    Ok(())
}

#[test]
fn bare_auth_logout_with_duplicate_env_defaults_is_not_ambiguous() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand_with(
        ["auth", "logout"],
        None,
        &default_index_env("file:///srv/index,file:///srv/index"),
    )?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains(
            "Logging out from index `file:///srv/index`",
        ))
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    Ok(())
}

#[test]
fn bare_auth_logout_resolves_a_configured_default_index() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let config_path = cwd.join("sysand.toml");
    fs::write(
        &config_path,
        "[[index]]\nurl = \"file:///srv/cfg-index\"\ndefault = true\n",
    )?;
    let out = run_sysand_in(&cwd, ["auth", "logout"], Some(config_path.as_str()))?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains(
            "Logging out from index `file:///srv/cfg-index`",
        ))
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    Ok(())
}

#[test]
fn bare_auth_logout_with_two_configured_defaults_asks_for_an_explicit_url() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let config_path = cwd.join("sysand.toml");
    fs::write(
        &config_path,
        "[[index]]\nurl = \"https://a.example\"\ndefault = true\n\n\
         [[index]]\nurl = \"https://b.example\"\ndefault = true\n",
    )?;
    let out = run_sysand_in(&cwd, ["auth", "logout"], Some(config_path.as_str()))?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("more than one default index"))
        .stderr(predicate::str::contains(AMBIGUOUS_MESSAGE));
    Ok(())
}

#[test]
fn auth_status_succeeds_without_any_credentials() -> TestResult {
    let (_temp_dir, _cwd, out) = run_sysand_with(["auth", "status"], None, &no_env())?;
    out.assert().success().stdout(predicate::str::contains(
        "No `SYSAND_CRED_*` environment credentials.",
    ));
    Ok(())
}

#[test]
fn auth_status_lists_env_credentials_and_never_secrets() -> TestResult {
    let mut env = IndexMap::new();
    env.insert(
        "SYSAND_CRED_TEST".to_string(),
        "https://example.com/**".to_string(),
    );
    env.insert(
        "SYSAND_CRED_TEST_BEARER_TOKEN".to_string(),
        "super-secret-token".to_string(),
    );
    env.insert(
        "SYSAND_CRED_BSC".to_string(),
        "https://basic.example/**".to_string(),
    );
    env.insert(
        "SYSAND_CRED_BSC_BASIC_USER".to_string(),
        "secret-user-name".to_string(),
    );
    env.insert(
        "SYSAND_CRED_BSC_BASIC_PASS".to_string(),
        "secret-pass-word".to_string(),
    );
    // A pattern without any scheme variable: the eager auth-policy build
    // rejects this, but `auth status` must stay usable to diagnose it.
    env.insert(
        "SYSAND_CRED_LONELY".to_string(),
        "https://lonely.example/**".to_string(),
    );

    let (_temp_dir, _cwd, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "env     SYSAND_CRED_TEST  https://example.com/**",
        ))
        .stdout(predicate::str::contains(
            "env     SYSAND_CRED_BSC  https://basic.example/**",
        ))
        .stdout(predicate::str::contains(
            "env     SYSAND_CRED_LONELY  https://lonely.example/**",
        ))
        .stdout(predicate::str::contains("super-secret-token").not())
        .stdout(predicate::str::contains("secret-user-name").not())
        .stdout(predicate::str::contains("secret-pass-word").not());
    Ok(())
}
