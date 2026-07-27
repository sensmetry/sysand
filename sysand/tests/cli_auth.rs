// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI tests for `sysand auth status` and `sysand auth logout`.
//!
//! These tests exercise argument parsing, the default-index resolution
//! chain, and status rendering of `SYSAND_CRED_*` entries. Logout tests
//! deliberately fail before any credential store access (parse errors,
//! non-HTTP(S) targets, ambiguous defaults), so no test requires, or can
//! ever mutate, a real OS keyring: store behavior itself is covered by the
//! core tests over an in-memory blob backend.

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
fn auth_status_with_nothing_configured_prints_a_single_line() -> TestResult {
    // The seam store file does not exist: no stored credentials, no env
    // credentials. The two per-source negatives collapse into exactly
    // one combined line.
    let (_store_dir, store_path) = seam_store()?;
    let (_temp_dir, _cwd, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert().success().stdout(predicate::eq(
        "No credentials configured (no stored credentials, no `SYSAND_CRED_*` variables).\n",
    ));
    Ok(())
}

#[test]
fn auth_status_lists_env_credentials_and_never_secrets() -> TestResult {
    // Seam store file does not exist: env credentials only.
    let (_store_dir, store_path) = seam_store()?;
    let mut env = seam_env(&store_path);
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
        // The leading spaces are the 12-column gutter, part of plain
        // piped output (as in all alignment predicates in this file).
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_TEST  https://example.com/**",
        ))
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_BSC  https://basic.example/**",
        ))
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_LONELY  https://lonely.example/**",
        ))
        .stdout(predicate::str::contains("super-secret-token").not())
        .stdout(predicate::str::contains("secret-user-name").not())
        .stdout(predicate::str::contains("secret-pass-word").not())
        // Env credentials exist, so the stored-side negative is omitted
        // entirely (no "no stored credentials" noise, no combined negative).
        .stdout(predicate::str::contains("No credentials configured").not())
        .stdout(predicate::str::contains("Stored").not());
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
        // The echo is printed (stdout, shown even under `--quiet`); the
        // result lines ("Stored", "Covers") are log output (stderr).
        .stdout(predicate::str::contains(
            "  Logging in to index `http://127.0.0.1:1/`",
        ))
        .stderr(predicate::str::contains("Covers http://127.0.0.1:1/**"))
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
        .stdout(predicate::str::contains("      Stored http://127.0.0.1:1/"))
        .stdout(predicate::str::contains(
            "             covers: http://127.0.0.1:1/**",
        ))
        // A stored login exists, so the env-side negative is omitted.
        .stdout(predicate::str::contains("SYSAND_CRED").not());
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
        .stderr(predicate::str::contains("Replacing existing credential").not());

    let (_t, _c, out) = run_sysand_stdin(
        // A different spelling of the same index normalizes to one key.
        ["auth", "login", "--token-stdin", "http://127.0.0.1:1/"],
        &env,
        b"new-tok\n",
    )?;
    // The notice is informational (log output, stderr), not stdout: the
    // stdout channel carries only the pre-prompt target echo.
    out.assert().success().stderr(predicate::str::contains(
        "   Replacing existing credential for `http://127.0.0.1:1/`",
    ));

    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"new-tok""#), "blob was: {blob}");
    assert!(!blob.contains("old-tok"), "blob was: {blob}");

    // Under `--quiet` a re-login is exit-code-only apart from the target
    // echo: the replacement notice is suppressed with the result lines.
    let (_t, _c, out) = run_sysand_stdin(
        [
            "--quiet",
            "auth",
            "login",
            "--token-stdin",
            "http://127.0.0.1:1",
        ],
        &env,
        b"third-tok\n",
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("Logging in to index"))
        .stderr(predicate::str::contains("Replacing").not())
        .stderr(predicate::str::contains("Stored").not());
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
        .stdout(predicate::str::contains("No credentials configured"));
    Ok(())
}

#[test]
fn auth_logout_of_an_unknown_index_warns_and_succeeds() -> TestResult {
    // The seam store file does not exist: nothing is stored. Logout is
    // idempotent: a warning and exit 0, so cleanup scripts need not
    // swallow a failure. The warning still points at `auth status`,
    // whose listing shows every stored key in the exact spelling logout
    // accepts (the fix for a typoed URL).
    let (_store_dir, store_path) = seam_store()?;
    let (_t, _c, out) = run_sysand_with(
        ["auth", "logout", "https://nothing.example"],
        None,
        &seam_env(&store_path),
    )?;
    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: no stored credential for `https://nothing.example/`",
        ))
        .stderr(predicate::str::contains(
            "run `sysand auth status` to list the stored logins and their exact keys",
        ));
    Ok(())
}

#[test]
fn auth_login_stored_anyway_hedges_a_read_404() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    let _config = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"api_root": "{}/api/"}}"#, server.url()))
        .create();
    // The read surface answers 404 with and without the token. A 404 can
    // mean a rejected token (GitLab-style hosts) or no `index.json` at
    // all, so the stored-anyway warning must carry the same hedge the
    // refusal message has. The advertised API accepts, so the login
    // stores the credential.
    let _index = server
        .mock("GET", "/index.json")
        .with_status(404)
        .expect(2)
        .create();
    let _whoami = server
        .mock("GET", "/api/v1/whoami")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(WHOAMI_BODY)
        .create();

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &server.url()],
        &env,
        b"tok\n",
    )?;
    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "HTTP 404, which can also mean no index exists at this URL",
        ))
        .stderr(predicate::str::contains("stored anyway"))
        .stderr(predicate::str::contains("validated (api)"));
    Ok(())
}

#[test]
fn auth_login_covers_a_disjoint_api_root_from_discovery() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    // A second server stands in for the disjoint API host, so the
    // advertised-api whoami probe has somewhere real to go.
    let mut api_server = mockito::Server::new();
    let api_root = format!("{}/base/", api_server.url());
    let _mock = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"api_root": "{api_root}"}}"#))
        .create();
    let _index = server.mock("GET", "/index.json").with_status(200).create();
    let _whoami = api_server
        .mock("GET", "/base/v1/whoami")
        .match_header("authorization", "Bearer tok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(WHOAMI_BODY)
        .create();
    let root = format!("{}/", server.url());

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &server.url()],
        &env,
        b"tok\n",
    )?;
    out.assert()
        .success()
        // The "Covers" globs are part of the login result (log output).
        .stderr(predicate::str::contains(format!("{root}**")))
        .stderr(predicate::str::contains(format!("{api_root}**")))
        // Public read never exercised the token; the advertised API did.
        .stderr(predicate::str::contains("validated (api)"));

    // The scoped claim is persisted and rendered by status.
    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "      Stored {root}  validated (api)"
        )));
    Ok(())
}

#[test]
fn auth_login_refuses_a_credential_every_exercised_surface_rejected() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    let _config = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .create();
    // Unauth baseline 401, forced bearer retry 401: rejected everywhere
    // it was exercised, so the login must refuse and store nothing.
    let _index = server
        .mock("GET", "/index.json")
        .with_status(401)
        .expect(2)
        .create();

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &server.url()],
        &env,
        b"bad-tok\n",
    )?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("rejected"))
        .stderr(predicate::str::contains("nothing was stored"));
    assert!(
        !store_path.exists(),
        "a refused login must not write the store"
    );
    Ok(())
}

#[test]
fn auth_login_validates_the_read_surface_of_a_private_index() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    let _config = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .create();
    let _unauth = server
        .mock("GET", "/index.json")
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(401)
        .create();
    let _forced = server
        .mock("GET", "/index.json")
        .match_header("authorization", "Bearer sekrit-tok")
        .with_status(200)
        .create();

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &server.url()],
        &env,
        b"sekrit-tok\n",
    )?;
    out.assert()
        .success()
        // Scoped claim: validated on the read surface only.
        .stderr(predicate::str::contains("validated (read)"));
    let blob = fs::read_to_string(&store_path)?;
    assert!(
        blob.contains(r#""secret":"sekrit-tok""#),
        "blob was: {blob}"
    );

    // The claim was persisted and status renders it per entry.
    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "      Stored {}/  validated (read)",
            server.url()
        )));
    Ok(())
}

#[test]
fn auth_login_status_logout_round_trip_a_template_target() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    let mut server = mockito::Server::new();
    let template = format!("{}/repo/files/{{path}}/raw?ref=index", server.url());
    let anchor_glob = format!("{}/repo/files/**", server.url());
    let ref_query = mockito::Matcher::UrlEncoded("ref".into(), "index".into());
    let _config = server
        .mock("GET", "/repo/files/sysand-index-config.json/raw")
        .match_query(ref_query.clone())
        .with_status(404)
        .create();
    // The GitLab reality: unauthenticated GET answers 404 on a private
    // repo, the forced bearer retry 200s (the accepted-read path).
    let _unauth = server
        .mock("GET", "/repo/files/index.json/raw")
        .match_query(ref_query.clone())
        .match_header("authorization", mockito::Matcher::Missing)
        .with_status(404)
        .create();
    let _forced = server
        .mock("GET", "/repo/files/index.json/raw")
        .match_query(ref_query)
        .match_header("authorization", "Bearer template-tok")
        .with_status(200)
        .create();

    let (_t, _c, out) = run_sysand_stdin(
        ["auth", "login", "--token-stdin", &template],
        &env,
        b"template-tok\n",
    )?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Logging in to index `{template}`"
        )))
        // The "Covers" glob is part of the login result (log output).
        .stderr(predicate::str::contains(&anchor_glob))
        .stderr(predicate::str::contains("validated (read)"));

    // `status` shows the key in the exact form `logout` accepts.
    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!("      Stored {template}")))
        .stdout(predicate::str::contains(&anchor_glob));

    let (_t, _c, out) = run_sysand_with(["auth", "logout", &template], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            " Logging out from index `{template}`"
        )));

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("No credentials configured"));
    Ok(())
}

#[test]
fn auth_status_shows_identity_learned_by_a_validating_login() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);

    // A blob as a validating login writes it (identity from whoami).
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://example.com/",
            "globs":["https://example.com/**"],
            "scheme":"bearer",
            "secret":"sekrit-status-tok",
            "expires_at":"2999-09-01T00:00:00Z",
            "subject":{"type":"user","name":"alice"},
            "token_name":"laptop",
            "token_prefix":"sysand_u_1a2b3c4d"}]}"#,
    )?;

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("subject: user alice"))
        .stdout(predicate::str::contains("token prefix: sysand_u_1a2b3c4d"))
        .stdout(predicate::str::contains("expires in"))
        // A pre-B12 blob has no `validated` field: shown as the
        // security-relevant "not validated", never a silent default.
        .stdout(predicate::str::contains("not validated"))
        .stdout(predicate::str::contains("sekrit-status-tok").not());
    Ok(())
}

#[test]
fn auth_status_renders_one_stored_entry_without_separators_or_negatives() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://one.example/",
            "globs":["https://one.example/**"],
            "scheme":"bearer",
            "secret":"tok-one",
            "validated":["read","api"]}]}"#,
    )?;

    // Exact plain output: no blank line around a single entry, and no
    // negative for the empty env side.
    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert().success().stdout(predicate::eq(concat!(
        "      Stored https://one.example/  validated (read, api)\n",
        "             covers: https://one.example/**\n",
    )));
    Ok(())
}

#[test]
fn auth_status_separates_multiple_stored_entries_with_a_blank_line() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://a.example/",
            "globs":["https://a.example/**"],
            "scheme":"bearer",
            "secret":"tok-a"
        },{
            "key":"https://b.example/",
            "globs":["https://b.example/**"],
            "scheme":"bearer",
            "secret":"tok-b",
            "validated":["read"]}]}"#,
    )?;

    // Exact plain output: one blank line between the entries, none after
    // the last.
    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert().success().stdout(predicate::eq(concat!(
        "      Stored https://a.example/  not validated\n",
        "             covers: https://a.example/**\n",
        "\n",
        "      Stored https://b.example/  validated (read)\n",
        "             covers: https://b.example/**\n",
    )));
    Ok(())
}

#[test]
fn auth_status_with_both_sources_lists_both_without_negatives() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://one.example/",
            "globs":["https://one.example/**"],
            "scheme":"bearer",
            "secret":"tok-one"}]}"#,
    )?;
    let mut env = seam_env(&store_path);
    env.insert(
        "SYSAND_CRED_CI".to_string(),
        "https://ci.example/**".to_string(),
    );

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "      Stored https://one.example/",
        ))
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_CI  https://ci.example/**",
        ))
        .stdout(predicate::str::contains("No credentials configured").not());
    Ok(())
}

// `(default index)` marker

const MARKER: &str = "(default index)";

/// A seam blob with one stored entry for `https://one.example/`.
fn one_example_blob() -> &'static str {
    r#"{"version":1,"credentials":[{
        "key":"https://one.example/",
        "globs":["https://one.example/**"],
        "scheme":"bearer",
        "secret":"tok-one",
        "validated":["read"]}]}"#
}

#[test]
fn auth_status_marks_the_entry_for_a_configured_default_index() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    fs::write(&store_path, one_example_blob())?;
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let config_path = cwd.join("sysand.toml");
    fs::write(
        &config_path,
        "[[index]]\nurl = \"https://one.example\"\ndefault = true\n",
    )?;

    let out = run_sysand_in_with(
        &cwd,
        ["auth", "status"],
        Some(config_path.as_str()),
        &seam_env(&store_path),
    )?;
    out.assert().success().stdout(predicate::str::contains(
        "      Stored https://one.example/  validated (read)  (default index)",
    ));
    Ok(())
}

#[test]
fn auth_status_marks_the_entry_for_an_env_default_index() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    fs::write(&store_path, one_example_blob())?;
    let mut env = seam_env(&store_path);
    env.insert(
        "SYSAND_DEFAULT_INDEX".to_string(),
        "https://one.example".to_string(),
    );

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert().success().stdout(predicate::str::contains(
        "      Stored https://one.example/  validated (read)  (default index)",
    ));
    Ok(())
}

#[test]
fn auth_status_marks_the_entry_for_the_built_in_default_index() -> TestResult {
    // No override, no configured default: the chain falls through to the
    // built-in `https://sysand.com`, whose normalized key must match.
    let (_store_dir, store_path) = seam_store()?;
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://sysand.com/",
            "globs":["https://sysand.com/**"],
            "scheme":"bearer",
            "secret":"tok"}]}"#,
    )?;

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert().success().stdout(predicate::str::contains(
        "      Stored https://sysand.com/  not validated  (default index)",
    ));
    Ok(())
}

#[test]
fn auth_status_marks_a_stored_entry_whose_glob_covers_the_default_index() -> TestResult {
    // Keyed elsewhere, but a disjoint glob (the `api_root` shape) covers
    // the built-in default index root: marked without key equality.
    let (_store_dir, store_path) = seam_store()?;
    fs::write(
        &store_path,
        r#"{"version":1,"credentials":[{
            "key":"https://other.example/",
            "globs":["https://other.example/**","https://sysand.com/**"],
            "scheme":"bearer",
            "secret":"tok"}]}"#,
    )?;

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert().success().stdout(predicate::str::contains(
        "      Stored https://other.example/  not validated  (default index)",
    ));
    Ok(())
}

#[test]
fn auth_status_marks_an_env_entry_whose_pattern_covers_the_default_index() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let mut env = seam_env(&store_path);
    env.insert(
        "SYSAND_CRED_HIT".to_string(),
        "https://sysand.com/**".to_string(),
    );
    env.insert(
        "SYSAND_CRED_MISS".to_string(),
        "https://elsewhere.example/**".to_string(),
    );

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_HIT  https://sysand.com/**  (default index)",
        ))
        .stdout(predicate::str::contains(
            "         Env SYSAND_CRED_MISS  https://elsewhere.example/**\n",
        ));
    Ok(())
}

#[test]
fn auth_status_with_an_ambiguous_default_chain_notes_and_marks_nothing() -> TestResult {
    // Bare `login`/`logout` would error here; status is diagnostic and
    // must still succeed, with a note instead of a marker.
    let (_store_dir, store_path) = seam_store()?;
    fs::write(&store_path, one_example_blob())?;
    let mut env = seam_env(&store_path);
    env.insert(
        "SYSAND_DEFAULT_INDEX".to_string(),
        "https://one.example,https://two.example".to_string(),
    );

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "note: more than one default index is configured; no entry is marked as \
             the default index",
        ))
        .stdout(predicate::str::contains(MARKER).not());
    Ok(())
}

#[test]
fn auth_status_without_a_matching_entry_shows_no_marker() -> TestResult {
    // A stored entry that neither is nor covers the built-in default:
    // output is unchanged, no marker, no note.
    let (_store_dir, store_path) = seam_store()?;
    fs::write(&store_path, one_example_blob())?;

    let (_t, _c, out) = run_sysand_with(["auth", "status"], None, &seam_env(&store_path))?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(MARKER).not())
        .stdout(predicate::str::contains("note:").not());
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

#[test]
fn auth_login_without_keyring_distinguishes_ports_in_the_env_stem() -> TestResult {
    // Two indexes on different ports of one host must get different
    // variable stems, or the second guidance would silently overwrite
    // the first credential.
    let mut env = IndexMap::new();
    env.insert(SEAM_ENV_VAR.to_string(), ":absent:".to_string());

    let (_t, _c, out) = run_sysand_stdin(
        [
            "auth",
            "login",
            "--token-stdin",
            "https://sysand.example:8443/idx",
        ],
        &env,
        b"tok\n",
    )?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains(
            "SYSAND_CRED_SYSAND_EXAMPLE_8443=https://sysand.example:8443/idx/**",
        ))
        .stdout(predicate::str::contains(
            "SYSAND_CRED_SYSAND_EXAMPLE_8443_BEARER_TOKEN=<token>",
        ));
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
    // The hidden prompt must not echo the secret: capture everything
    // printed between sending the token and the stored confirmation, and
    // assert the token never appears there (the only place it may appear
    // is the seam store file).
    let after_prompt = session.exp_string(STORED_MESSAGE)?;
    assert!(
        !after_prompt.contains("pty-tok"),
        "secret was echoed to the terminal: {after_prompt:?}"
    );
    assert!(await_exit(session)?.success());

    let blob = fs::read_to_string(&store_path)?;
    assert!(blob.contains(r#""secret":"pty-tok""#), "blob was: {blob}");
    Ok(())
}

// `sysand auth whoami`
//
// Query-only live identity check against `api_root/v1/whoami`. Discovery
// and the whoami request go to a local mockito server; credentials come
// from the seam store or `SYSAND_CRED_*` variables, never a real keyring.

const WHOAMI_BODY: &str = r#"{
    "subject": {"type": "user", "name": "alice"},
    "token": {"name": "laptop", "prefix": "sysand_u_1a2b3c4d",
              "expires_at": "2999-09-01T00:00:00Z"}}"#;

/// Discovery answering with an advertised `api_root` under `/api/` on the
/// same server.
fn mock_api_discovery(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock("GET", "/sysand-index-config.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"api_root": "{}/api/"}}"#, server.url()))
        .create()
}

/// A seam store blob holding one stored login for the server root.
fn seed_stored_login(
    store_path: &camino::Utf8Path,
    server_url: &str,
    secret: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(
        store_path,
        format!(
            r#"{{"version":1,"credentials":[{{
                "key":"{server_url}/",
                "globs":["{server_url}/**"],
                "scheme":"bearer",
                "secret":"{secret}"}}]}}"#
        ),
    )?;
    Ok(())
}

#[test]
fn auth_whoami_with_a_stored_login_renders_the_identity() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);
    let mut server = mockito::Server::new();
    let _config = mock_api_discovery(&mut server);
    let _whoami = server
        .mock("GET", "/api/v1/whoami")
        .match_header("authorization", "Bearer sekrit-whoami-tok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(WHOAMI_BODY)
        .create();
    seed_stored_login(&store_path, &server.url(), "sekrit-whoami-tok")?;

    let (_t, _c, out) = run_sysand_with(["auth", "whoami", &server.url()], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "    Checking identity on index `{}/`",
            server.url()
        )))
        .stdout(predicate::str::contains(format!(
            "       Using stored credential for `{}/`",
            server.url()
        )))
        .stdout(predicate::str::contains("     Subject user alice"))
        .stdout(predicate::str::contains("  Token name laptop"))
        .stdout(predicate::str::contains("Token prefix sysand_u_1a2b3c4d"))
        .stdout(predicate::str::contains("expires in"))
        .stdout(predicate::str::contains("sekrit-whoami-tok").not());
    Ok(())
}

#[test]
fn auth_whoami_reports_a_rejected_credential_with_the_source() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);
    let mut server = mockito::Server::new();
    let _config = mock_api_discovery(&mut server);
    let _whoami = server
        .mock("GET", "/api/v1/whoami")
        .with_status(401)
        .create();
    seed_stored_login(&store_path, &server.url(), "stale-tok")?;

    let (_t, _c, out) = run_sysand_with(["auth", "whoami", &server.url()], None, &env)?;
    out.assert()
        .failure()
        .stdout(predicate::str::contains("Using stored credential for"))
        .stderr(predicate::str::contains("rejected the credential"))
        .stderr(predicate::str::contains("sysand auth login"));
    Ok(())
}

#[test]
fn auth_whoami_env_credential_wins_over_a_stored_login() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let mut server = mockito::Server::new();
    let mut env = seam_env(&store_path);
    env.insert(
        "SYSAND_CRED_WTEST".to_string(),
        format!("{}/**", server.url()),
    );
    env.insert(
        "SYSAND_CRED_WTEST_BEARER_TOKEN".to_string(),
        "env-tok".to_string(),
    );
    let _config = mock_api_discovery(&mut server);
    // The mock only matches the env token: a stored-token request would
    // 501, so success proves source precedence.
    let _whoami = server
        .mock("GET", "/api/v1/whoami")
        .match_header("authorization", "Bearer env-tok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(WHOAMI_BODY)
        .create();
    seed_stored_login(&store_path, &server.url(), "shadowed-stored-tok")?;

    let (_t, _c, out) = run_sysand_with(["auth", "whoami", &server.url()], None, &env)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "Using credential from `SYSAND_CRED_WTEST`",
        ))
        .stdout(predicate::str::contains("Subject user alice"));
    Ok(())
}

#[test]
fn auth_whoami_without_a_matching_credential_suggests_login() -> TestResult {
    // The seam store file does not exist: no stored credentials.
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);
    let mut server = mockito::Server::new();
    let _config = mock_api_discovery(&mut server);

    let (_t, _c, out) = run_sysand_with(["auth", "whoami", &server.url()], None, &env)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("no credential matches"))
        // The hint leads with the interactive fix and names the
        // `SYSAND_CRED_*` variables second, as the CI path.
        .stderr(predicate::str::contains(format!(
            "run `sysand auth login {}/` to store a credential; \
             in CI, set `SYSAND_CRED_*` environment variables instead",
            server.url()
        )));
    Ok(())
}

#[test]
fn auth_whoami_errors_when_the_index_does_not_advertise_an_api() -> TestResult {
    let (_store_dir, store_path) = seam_store()?;
    let env = seam_env(&store_path);
    let mut server = mockito::Server::new();
    // No discovery document: the plain-URL runtime default is not an
    // advertised API, so there is nothing to ask.
    let _config = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .create();
    seed_stored_login(&store_path, &server.url(), "tok")?;

    let (_t, _c, out) = run_sysand_with(["auth", "whoami", &server.url()], None, &env)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("does not advertise an API"));
    Ok(())
}

#[test]
fn auth_whoami_rejects_a_non_http_index() -> TestResult {
    // Fails during target validation, before any credential store access.
    let (_temp_dir, _cwd, out) = run_sysand(["auth", "whoami", "file:///srv/index"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(NOT_HTTP_MESSAGE));
    Ok(())
}
