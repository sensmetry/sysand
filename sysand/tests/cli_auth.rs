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

// `sysand auth login`
//
// These tests run against the debug-only credential store seam
// (`SYSAND_TEST_CREDENTIAL_STORE`, sysand/src/credential_store.rs): a
// file-backed blob store, or a simulated-absent backend, so no test can
// ever touch the real OS keyring.

const SEAM_ENV_VAR: &str = "SYSAND_TEST_CREDENTIAL_STORE";
const NO_TTY_MESSAGE: &str = "no terminal for prompt; pass the token with `--token-stdin`";
const STORED_MESSAGE: &str = "stored, not validated";

fn seam_env(store_path: &camino::Utf8Path) -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    env.insert(SEAM_ENV_VAR.to_string(), store_path.to_string());
    env
}

/// Run sysand with piped stdin (so stdin is not a TTY) and the given
/// bytes written to it.
fn run_sysand_stdin<'a, I: IntoIterator<Item = &'a str>>(
    args: I,
    env: &IndexMap<String, String>,
    input: &[u8],
) -> Result<
    (
        camino_tempfile::Utf8TempDir,
        camino::Utf8PathBuf,
        std::process::Output,
    ),
    Box<dyn std::error::Error>,
> {
    use std::io::Write as _;
    let (temp_dir, cwd, mut cmd) = sysand_cmd(args, None, env)?;
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input)?;
    Ok((temp_dir, cwd, child.wait_with_output()?))
}

/// A temp home for the seam store file, outliving the commands under test.
fn seam_store()
-> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let dir = camino_tempfile::Utf8TempDir::with_prefix("sysand_cred_seam_")?;
    let path = dir.path().join("creds.json");
    Ok((dir, path))
}

#[test]
fn auth_login_token_stdin_stores_and_status_lists_the_entry() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    // Port 1 answers nothing, so discovery falls back to the URL-derived
    // pattern with a warning; no network beyond localhost is touched.
    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
        &env,
        b"sekrit-tok\n",
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "Logging in to index `http://127.0.0.1:1/`",
        ))
        .stdout(predicate::str::contains("http://127.0.0.1:1/**"))
        // The styled confirmation is log output (stderr).
        .stderr(predicate::str::contains(STORED_MESSAGE));

    // The stored secret is exactly the piped bytes minus one trailing
    // newline (observable only through the seam file; `status` never
    // shows secrets).
    let blob = fs::read_to_string(&store_path)?;
    assert!(
        blob.contains(r#""secret":"sekrit-tok""#),
        "blob was: {blob}"
    );

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("stored  http://127.0.0.1:1/"))
        .stdout(predicate::str::contains("patterns: http://127.0.0.1:1/**"));
    Ok(())
}

#[test]
fn auth_login_token_stdin_trims_exactly_one_trailing_newline() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    // CRLF counts as one newline.
    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
        &env,
        b"crlf-tok\r\n",
    )?;
    out.assert().success();
    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"crlf-tok""#), "blob was: {blob}");

    // Only one trailing newline is trimmed; an inner one is kept.
    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
        &env,
        b"odd-tok\n\n",
    )?;
    out.assert().success();
    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"odd-tok\n""#), "blob was: {blob}");
    Ok(())
}

#[test]
fn auth_login_twice_reports_the_replacement_and_overwrites() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
        &env,
        b"old-tok\n",
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("replacing existing credential").not());

    let (_t, _c, out) = run_sysand_stdin(
        // A different spelling of the same index normalizes to one key.
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1/"],
        &env,
        b"new-tok\n",
    )?;
    out.assert().success().stdout(predicate::str::contains(
        "replacing existing credential for `http://127.0.0.1:1/`",
    ));

    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"new-tok""#), "blob was: {blob}");
    assert!(!blob.contains("old-tok"), "blob was: {blob}");
    Ok(())
}

#[test]
fn auth_login_then_logout_removes_the_stored_entry() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
        &env,
        b"tok\n",
    )?;
    out.assert().success();

    let (_t, _c, out) = run_sysand_with(["auth", "logout", "http://127.0.0.1:1"], None, &env)?;
    out.assert().success();

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("No stored index logins."));
    Ok(())
}

#[test]
fn auth_login_covers_a_disjoint_api_root_from_discovery() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    let _mock = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"api_root": "https://api.example.com/base/"}"#)
        .create();
    let root = format!("{}/", server.url());

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &server.url()],
        &env,
        b"tok\n",
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!("{root}**")))
        .stdout(predicate::str::contains("https://api.example.com/base/**"));
    Ok(())
}

#[test]
fn auth_login_fails_fast_when_stdin_is_not_a_terminal() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    // `run_sysand_with` runs with a null stdin: not a TTY.
    let (_t, _c, out) = run_sysand_with(
        ["auth", "login", "http://127.0.0.1:1"],
        None,
        &seam_env(&store_path),
    )?;
    out.assert()
        .failure()
        // The resolved index is echoed even though no secret was read.
        .stdout(predicate::str::contains(
            "Logging in to index `http://127.0.0.1:1/`",
        ))
        .stderr(predicate::str::contains(NO_TTY_MESSAGE));
    assert!(!store_path.exists(), "nothing may be stored");
    Ok(())
}

#[test]
fn auth_login_rejects_an_empty_token() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    for input in [b"".as_slice(), b"\n".as_slice()] {
        let (_t, _c, out) = run_sysand_stdin(
            ["auth", "login", "--token-stdin", "http://127.0.0.1:1"],
            &seam_env(&store_path),
            input,
        )?;
        out.assert()
            .failure()
            .stderr(predicate::str::contains("empty token"));
    }
    assert!(!store_path.exists(), "nothing may be stored");
    Ok(())
}

#[test]
fn auth_login_rejects_a_non_http_index_before_reading_a_secret() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", "file:///srv/index"],
        &seam_env(&store_path),
        b"tok\n",
    )?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    assert!(!store_path.exists(), "nothing may be stored");
    Ok(())
}

#[test]
fn auth_login_without_keyring_backend_prints_env_lines_with_a_placeholder() -> TestResult {
    let mut env = IndexMap::new();
    env.insert(SEAM_ENV_VAR.to_string(), ":absent:".to_string());

    let (_t, _c, out) = run_sysand_stdin(
        [
            "auth",
            "login",
            "--token-stdin",
            "https://sysand.example/idx",
        ],
        &env,
        b"hush-secret\n",
    )?;
    let assert = out
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "SYSAND_CRED_SYSAND_EXAMPLE=https://sysand.example/idx/**",
        ))
        .stdout(predicate::str::contains(
            "SYSAND_CRED_SYSAND_EXAMPLE_BEARER_TOKEN=<token>",
        ));
    // The entered secret must never be echoed anywhere.
    let output = assert.get_output();
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !String::from_utf8_lossy(stream).contains("hush-secret"),
            "the secret leaked into command output"
        );
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[test]
fn auth_login_prompts_hidden_on_a_terminal() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;

    let (_t, _c, mut session) = run_sysand_interactive_with(
        ["auth", "login", "http://127.0.0.1:1"],
        Some(30_000),
        None,
        &seam_env(&store_path),
    )?;
    session.exp_string("Logging in to index `http://127.0.0.1:1/`")?;
    session.exp_string("Enter token for `http://127.0.0.1:1/`:")?;
    session.send_line("pty-tok")?;
    session.exp_string(STORED_MESSAGE)?;
    assert!(await_exit(session)?.success());

    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"pty-tok""#), "blob was: {blob}");
    // The hidden prompt must not have echoed the secret; the only place
    // it may appear is the seam store file.
    Ok(())
}
