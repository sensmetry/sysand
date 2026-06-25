// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::fs;

use assert_cmd::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use indexmap::IndexMap;
use mockito::{Matcher, Server};
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn init_project(name: &str) -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.0.0",
            "--name",
            name,
            "--license",
            "MIT",
        ],
        None,
    )?;
    out.assert().success();
    let mut licenses = cwd.join("LICENSES");
    fs::create_dir(&licenses)?;
    licenses.push("MIT.txt");
    fs::File::create(licenses)?;
    Ok((temp_dir, cwd))
}

fn run_sysand_ok(cwd: &Utf8Path, args: &[&str], cfg: Option<&str>) -> TestResult {
    let out = run_sysand_in(cwd, args.iter().copied(), cfg)?;
    out.assert().success();
    Ok(())
}

fn include_basic_model(cwd: &Utf8Path) -> TestResult {
    std::fs::write(cwd.join("test.sysml"), "package P;\n")?;
    run_sysand_ok(cwd, &["include", "--no-index-symbols", "test.sysml"], None)?;
    run_sysand_ok(cwd, &["info", "metamodel", "--set", "sysml"], None)
}

fn build_default_kpar(cwd: &Utf8Path) -> TestResult {
    run_sysand_ok(cwd, &["build"], None)
}

fn build_kpar_at(cwd: &Utf8Path, kpar_path: &str) -> TestResult {
    run_sysand_ok(cwd, &["build", kpar_path], None)
}

fn setup_built_project(
    name: &str,
) -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd) = init_project(name)?;
    include_basic_model(&cwd)?;
    build_default_kpar(&cwd)?;
    Ok((temp_dir, cwd))
}

fn setup_built_project_at(
    name: &str,
    kpar_path: &str,
) -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd) = init_project(name)?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, kpar_path)?;
    Ok((temp_dir, cwd))
}

fn set_project_field(cwd: &Utf8Path, field: &str, value: &str) -> TestResult {
    run_sysand_ok(cwd, &["info", field, "--set", value], None)
}

fn bearer_env_for_url(url: &str) -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_TEST".to_string(), format!("{url}/**"));
    env.insert(
        "SYSAND_CRED_TEST_BEARER_TOKEN".to_string(),
        "test-token".to_string(),
    );
    env
}

/// Register a `sysand-index-config.json` mock that tells the client the
/// `api_root` is at `<server>/api/`. The upload fixtures in this file
/// POST to `/api/v1/upload`, so the advertised `api_root` must carry
/// the `/api/` segment. Discovery is mandatory on first use; without
/// this mock the discovery fetch would 404 and `api_root` would default
/// to the discovery root (yielding `/v1/upload`), which doesn't match
/// the mocks in this file.
fn mock_index_config_api_at_api(server: &mut Server) -> mockito::Mock {
    let body = format!(r#"{{"api_root":"{}/api/"}}"#, server.url());
    server
        .mock("GET", "/sysand-index-config.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .expect(1)
        .create()
}

fn mock_oidc_exchange(
    server: &mut Server,
    provider_token: &str,
    index_token: &str,
) -> mockito::Mock {
    server
        .mock("POST", "/api/v1/oidc/token")
        .match_header("content-type", "application/json")
        .match_body(Matcher::JsonString(format!(
            r#"{{"token":"{provider_token}"}}"#
        )))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(r#"{{"token":"{index_token}"}}"#))
        .expect(1)
        .create()
}

fn mock_publish_with_bearer(server: &mut Server, token: &str) -> mockito::Mock {
    server
        .mock("POST", "/api/v1/upload")
        .match_header("authorization", format!("Bearer {token}").as_str())
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create()
}

#[test]
fn publish_without_path_from_workspace_root_reports_explicit_error() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [{"path": "project1", "iris": ["urn:kpar:project1"]}]}"#,
    )?;
    std::fs::create_dir(cwd.join("project1"))?;

    let out = run_sysand_in(&cwd, ["publish", "--index", "http://localhost:1"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("not supported from a workspace"))
        .stderr(predicate::str::contains("explicit .kpar path"));

    Ok(())
}

#[test]
fn publish_missing_kpar() -> TestResult {
    let (_temp_dir, cwd) = init_project("test-publish")?;
    let out = run_sysand_in(&cwd, ["publish", "--index", "http://localhost:1"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("KPAR file not found"))
        .stderr(predicate::str::contains("sysand build"));

    Ok(())
}

#[test]
fn publish_explicit_missing_kpar() -> TestResult {
    let (_temp_dir, cwd) = init_project("test-publish")?;
    let out = run_sysand_in(
        &cwd,
        [
            "publish",
            "nonexistent.kpar",
            "--index",
            "http://localhost:1",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("KPAR file not found"));

    Ok(())
}

#[test]
fn publish_network_error() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        // After discovery, the network error surfaces through the
        // `sysand-index-config.json` fetch — the wording is
        // `HTTP request to \`…\` failed: …`, so match the stable
        // `HTTP request` prefix plus a separate `failed` assertion.
        .stderr(predicate::str::contains("HTTP request"))
        .stderr(predicate::str::contains("failed"));

    Ok(())
}

#[test]
fn publish_requires_index_argument() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    let out = run_sysand_in(&cwd, ["publish"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ))
        .stderr(predicate::str::contains("--index <URL>"));

    Ok(())
}

#[test]
fn publish_requires_index_value() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    let out = run_sysand_in(&cwd, ["publish", "--index"], None)?;
    out.assert().failure().stderr(predicate::str::contains(
        "a value is required for '--index <URL>'",
    ));

    Ok(())
}

#[test]
fn publish_requires_index_even_with_config_default() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;

    let config_path = cwd.join("publish-test.toml");
    std::fs::write(
        &config_path,
        "[[index]]\nurl = \"https://config-default.example.com\"\ndefault = true\n",
    )?;

    let out = run_sysand_in(&cwd, ["publish"], Some(config_path.as_str()))?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ))
        .stderr(predicate::str::contains("--index <URL>"));

    Ok(())
}

#[test]
fn publish_auto_uses_gitlab_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-auto-gitlab")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = mock_oidc_exchange(&mut server, "gitlab-oidc-token", "index-token");
    let publish_mock = mock_publish_with_bearer(&mut server, "index-token");

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;

    out.assert().success();
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_bare_trusted_publishing_flag_defaults_to_auto() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-bare-trusted-flag")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = mock_oidc_exchange(&mut server, "gitlab-oidc-token", "index-token");
    let publish_mock = mock_publish_with_bearer(&mut server, "index-token");

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--trusted-publishing",
            "--index",
            server.url().as_str(),
        ],
        None,
        &env,
    )?;

    out.assert().success();
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_forced_github_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-forced-github")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = mock_oidc_exchange(&mut server, "github-oidc-token", "index-token");
    let publish_mock = mock_publish_with_bearer(&mut server, "index-token");

    let mut github_server = Server::new();
    let github_mock = github_server
        .mock("GET", "/oidc")
        .match_header("authorization", "bearer github-request-token")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("existing".to_string(), "1".to_string()),
            Matcher::UrlEncoded("audience".to_string(), "sysand".to_string()),
        ]))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"value":"github-oidc-token"}"#)
        .expect(1)
        .create();

    let mut env = IndexMap::new();
    env.insert(
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN".to_string(),
        "github-request-token".to_string(),
    );
    env.insert(
        "ACTIONS_ID_TOKEN_REQUEST_URL".to_string(),
        format!("{}/oidc?existing=1", github_server.url()),
    );

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--trusted-publishing=github",
            "--index",
            server.url().as_str(),
        ],
        None,
        &env,
    )?;

    out.assert().success();
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();
    github_mock.assert();

    Ok(())
}

#[test]
fn publish_github_trusted_publishing_oidc_failure_errors() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-github-oidc-failure")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = server.mock("POST", "/api/v1/oidc/token").expect(0).create();
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let mut github_server = Server::new();
    let github_mock = github_server
        .mock("GET", "/oidc")
        .match_header("authorization", "bearer github-request-token")
        .match_query(Matcher::UrlEncoded(
            "audience".to_string(),
            "sysand".to_string(),
        ))
        .with_status(500)
        .with_body("runner error")
        .expect(1)
        .create();

    let mut env = IndexMap::new();
    env.insert(
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN".to_string(),
        "github-request-token".to_string(),
    );
    env.insert(
        "ACTIONS_ID_TOKEN_REQUEST_URL".to_string(),
        format!("{}/oidc", github_server.url()),
    );

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--trusted-publishing=github",
            "--index",
            server.url().as_str(),
        ],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("trusted publishing"))
        .stderr(predicate::str::contains("github"))
        .stderr(predicate::str::contains("HTTP status 500"));
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();
    github_mock.assert();

    Ok(())
}

#[test]
fn publish_space_separated_trusted_publishing_value_is_not_a_mode() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-trusted-space-value")?;
    let out = run_sysand_in(
        &cwd,
        [
            "publish",
            "--trusted-publishing",
            "never",
            "--index",
            "http://localhost:1",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("KPAR file not found at `never`"));

    Ok(())
}

#[test]
fn publish_trusted_publishing_never_preserves_no_bearer_error() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-trusted-never")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();
    let exchange_mock = server.mock("POST", "/api/v1/oidc/token").expect(0).create();

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--trusted-publishing=never",
            "--index",
            server.url().as_str(),
        ],
        None,
        &env,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "no bearer token credentials configured for publish URL",
    ));
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_explicit_bearer_wins_over_auto_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-bearer-wins")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = mock_publish_with_bearer(&mut server, "test-token");
    let exchange_mock = server.mock("POST", "/api/v1/oidc/token").expect(0).create();

    let mut env = bearer_env_for_url(server.url().as_str());
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;

    out.assert().success();
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_malformed_explicit_credentials_abort_before_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-malformed-explicit-creds")?;
    let mut env = IndexMap::new();
    env.insert(
        "SYSAND_CRED_TEST_BEARER_TOKEN".to_string(),
        "test-token".to_string(),
    );
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "please specify URL pattern SYSAND_CRED_TEST for credential",
        ))
        .stderr(predicate::str::contains("HTTP request").not());

    Ok(())
}

#[test]
fn publish_trusted_publishing_exchange_non_success_errors() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-exchange-error")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = server
        .mock("POST", "/api/v1/oidc/token")
        .with_status(403)
        .with_body("forbidden")
        .expect(1)
        .create();
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("trusted publishing"))
        .stderr(predicate::str::contains("HTTP status 403"));
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_trusted_publishing_exchange_malformed_response_errors() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-exchange-malformed")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = server
        .mock("POST", "/api/v1/oidc/token")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"not_token":"index-token"}"#)
        .expect(1)
        .create();
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_string(),
        "gitlab-oidc-token".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("trusted publishing"))
        .stderr(predicate::str::contains("malformed response"));
    publish_mock.assert();
    exchange_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_with_explicit_index_succeeds() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server
        .mock("POST", "/api/v1/upload")
        .match_header("authorization", "Bearer test-token")
        .match_header(
            "content-type",
            Matcher::Regex("multipart/form-data; boundary=.*".to_string()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_string()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="metadata""#.to_string()),
            Matcher::Regex(r#"Content-Type: application/json"#.to_string()),
            Matcher::Regex(r#""kpar_sha256_digest":"[0-9a-f]{64}""#.to_string()),
            Matcher::Regex(r#""normalized_publisher":"#.to_string()),
            Matcher::Regex(r#""normalized_name":"#.to_string()),
            Matcher::Regex(r#""version":"#.to_string()),
            Matcher::Regex(r#""license":"#.to_string()),
            Matcher::Regex(r#"name="kpar""#.to_string()),
            Matcher::Regex(r#"Content-Type: application/zip"#.to_string()),
        ]))
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create();

    let env = bearer_env_for_url(server.url().as_str());
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;
    out.assert().success();
    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_explicit_path_outside_project_dir() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project_at("outside-publish", "artifact.kpar")?;
    let kpar_path = cwd.join("artifact.kpar");

    let (_outside_temp_dir, outside_cwd) = new_temp_cwd()?;
    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &outside_cwd,
        [
            "publish",
            kpar_path.as_str(),
            "--index",
            "http://localhost:1",
        ],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("unable to find interchange project").not())
        // After discovery, the network error surfaces through the
        // `sysand-index-config.json` fetch — the wording is
        // `HTTP request to \`…\` failed: …`, so match the stable
        // `HTTP request` prefix plus a separate `failed` assertion.
        .stderr(predicate::str::contains("HTTP request"))
        .stderr(predicate::str::contains("failed"));

    Ok(())
}

#[test]
fn publish_invalid_index_url_errors_early() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project_at("invalid-index", "artifact.kpar")?;
    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "ftp://example.org"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("invalid index URL"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_rejects_upload_endpoint_index_url() -> TestResult {
    // If the user pastes the full upload URL as the `--index` value, the
    // pre-discovery shape check rejects it before discovery or upload.
    // The error message points the user back at the API root.
    let (_temp_dir, cwd) = setup_built_project_at("upload-endpoint-index", "artifact.kpar")?;
    let mut server = Server::new();
    let config_mock = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(0)
        .create();
    let publish_mock = server.mock("POST", "/v1/upload").expect(0).create();
    let endpoint_url = format!("{}/v1/upload", server.url());

    let env = bearer_env_for_url(server.url().as_str());
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", endpoint_url.as_str()],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("invalid index URL"))
        .stderr(predicate::str::contains("not the `v1/upload` endpoint"))
        .stderr(predicate::str::contains("HTTP request failed").not());
    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_rejects_invalid_semver_version() -> TestResult {
    let (_temp_dir, cwd) = init_project("invalid-version")?;

    let project_file = cwd.join(".project.json");
    let project_json = std::fs::read_to_string(&project_file)?;
    let project_json =
        project_json.replace("\"version\": \"1.0.0\"", "\"version\": \"not-semver\"");
    std::fs::write(project_file, project_json)?;

    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("`not-semver`"))
        .stderr(predicate::str::contains("Semantic Version"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_rejects_version_build_metadata() -> TestResult {
    let (_temp_dir, cwd) = init_project("version-build-metadata")?;

    // Manually set the field, since set_project_field runs the CLI
    let project_file = cwd.join(".project.json");
    let project_json = std::fs::read_to_string(&project_file)?;
    let project_json =
        project_json.replace("\"version\": \"1.0.0\"", "\"version\": \"1.2.3+build\"");
    std::fs::write(project_file, project_json)?;

    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("version `1.2.3+build`"))
        .stderr(predicate::str::contains("metadata"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_rejects_noncanonicalizable_publisher() -> TestResult {
    let (_temp_dir, cwd) = init_project("valid-publish-name")?;
    set_project_field(&cwd, "publisher", "bad__publisher")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("publisher `bad__publisher`"))
        .stderr(predicate::str::contains("must be 3-50 characters"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_rejects_noncanonicalizable_name() -> TestResult {
    let (_temp_dir, cwd) = init_project("valid-publish-name")?;
    set_project_field(&cwd, "name", "bad__name")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let env = bearer_env_for_url("http://localhost:1");
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
        &env,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("name `bad__name`"))
        .stderr(predicate::str::contains("must be 3-50 characters"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_sends_kpar_with_integrity_metadata() -> TestResult {
    let (_temp_dir, cwd) = init_project("seed-project")?;
    set_project_field(&cwd, "publisher", "Acme Labs")?;
    set_project_field(&cwd, "name", "My.Project Alpha")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server
        .mock("POST", "/api/v1/upload")
        .match_header("authorization", "Bearer test-token")
        .match_header(
            "content-type",
            Matcher::Regex("multipart/form-data; boundary=.*".to_string()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_string()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="metadata""#.to_string()),
            Matcher::Regex(r#""kpar_sha256_digest":"[0-9a-f]{64}""#.to_string()),
            Matcher::Regex(r#""normalized_publisher":"acme-labs""#.to_string()),
            Matcher::Regex(r#""normalized_name":"my.project-alpha""#.to_string()),
            Matcher::Regex(r#""version":"1.0.0""#.to_string()),
            Matcher::Regex(r#""license":"MIT""#.to_string()),
            Matcher::Regex(r#"name="kpar""#.to_string()),
            Matcher::Regex(r#"Content-Type: application/zip"#.to_string()),
        ]))
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create();

    let index_url = server.url();
    let env = bearer_env_for_url(index_url.as_str());
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "artifact.kpar", "--index", index_url.as_str()],
        None,
        &env,
    )?;

    out.assert().success();
    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_ignores_basic_auth_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-basic-auth-ignored")?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let pattern = format!("{}/**", server.url());
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_TEST".to_string(), pattern);
    env.insert(
        "SYSAND_CRED_TEST_BASIC_USER".to_string(),
        "user".to_string(),
    );
    env.insert(
        "SYSAND_CRED_TEST_BASIC_PASS".to_string(),
        "pass".to_string(),
    );

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "no bearer token credentials configured for publish URL",
        ))
        .stderr(predicate::str::contains("HTTP request failed").not());

    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_rejects_ambiguous_bearer_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-ambiguous-bearer")?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let base = server.url();
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_A".to_string(), format!("{base}/**"));
    env.insert(
        "SYSAND_CRED_A_BEARER_TOKEN".to_string(),
        "token-a".to_string(),
    );
    env.insert("SYSAND_CRED_B".to_string(), format!("{base}/api/**"));
    env.insert(
        "SYSAND_CRED_B_BEARER_TOKEN".to_string(),
        "token-b".to_string(),
    );

    let out = run_sysand_in_with(&cwd, ["publish", "--index", base.as_str()], None, &env)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "multiple bearer token credentials configured for publish URL",
        ))
        .stderr(predicate::str::contains("HTTP request failed").not());

    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

/// Helper for tests that publish to a mock server returning a specific status code
/// and assert that the CLI maps it to the expected error message(s).
fn assert_publish_error_status(
    project_name: &str,
    status: usize,
    response_body: &str,
    content_type: Option<&str>,
    expected_stderr: &[&str],
) -> TestResult {
    let (_temp_dir, cwd) = setup_built_project(project_name)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let mut mock = server
        .mock("POST", "/api/v1/upload")
        .with_status(status)
        .with_body(response_body)
        .expect(1);
    if let Some(ct) = content_type {
        mock = mock.with_header("content-type", ct);
    }
    let publish_mock = mock.create();

    let env = bearer_env_for_url(server.url().as_str());
    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;
    let mut assertion = out.assert().failure();
    for pattern in expected_stderr {
        assertion = assertion.stderr(predicate::str::contains(*pattern));
    }
    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_401_maps_to_auth_error() -> TestResult {
    assert_publish_error_status(
        "publish-auth-401",
        401,
        "unauthorized",
        None,
        &["authentication failed", "unauthorized"],
    )
}

#[test]
fn publish_403_maps_to_auth_error() -> TestResult {
    assert_publish_error_status(
        "publish-auth-403",
        403,
        "forbidden",
        None,
        &["authentication failed", "forbidden"],
    )
}

#[test]
fn publish_404_maps_to_not_found_error() -> TestResult {
    assert_publish_error_status(
        "publish-not-found",
        404,
        "missing endpoint",
        None,
        &["publish endpoint not found", "missing endpoint"],
    )
}

#[test]
fn publish_409_maps_to_conflict_error() -> TestResult {
    assert_publish_error_status(
        "publish-conflict",
        409,
        "already exists",
        None,
        &["conflict: package version already exists", "already exists"],
    )
}

#[test]
fn publish_500_json_error_body_extracts_error_message() -> TestResult {
    assert_publish_error_status(
        "publish-server-error",
        500,
        r#"{"error":"Invalid token"}"#,
        Some("application/json"),
        &["server error (500)", "Invalid token"],
    )
}
