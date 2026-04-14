// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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
    Ok((temp_dir, cwd))
}

fn run_sysand_ok(cwd: &Utf8Path, args: &[&str], cfg: Option<&str>) -> TestResult {
    let out = run_sysand_in(cwd, args.iter().copied(), cfg)?;
    out.assert().success();
    Ok(())
}

fn include_basic_model(cwd: &Utf8Path) -> TestResult {
    std::fs::write(cwd.join("test.sysml"), "package P;\n")?;
    run_sysand_ok(cwd, &["include", "--no-index-symbols", "test.sysml"], None)
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
        .stderr(predicate::str::contains("HTTP request failed"));

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
fn publish_with_explicit_index_succeeds() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    let mut server = Server::new();
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
        .stderr(predicate::str::contains("HTTP request failed"));

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
    let (_temp_dir, cwd) = setup_built_project_at("upload-endpoint-index", "artifact.kpar")?;
    let mut server = Server::new();
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();
    let endpoint_url = format!("{}/api/v1/upload", server.url());

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
        .stderr(predicate::str::contains("do not include `/api/v1/upload`"))
        .stderr(predicate::str::contains("HTTP request failed").not());
    publish_mock.assert();

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
        .stderr(predicate::str::contains("version field"))
        .stderr(predicate::str::contains("Semantic Versioning 2.0 version"))
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
        .stderr(predicate::str::contains("publisher field"))
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
        .stderr(predicate::str::contains("name field"))
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

    Ok(())
}

#[test]
fn publish_ignores_basic_auth_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-basic-auth-ignored")?;

    let mut server = Server::new();
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

    Ok(())
}

#[test]
fn publish_rejects_ambiguous_bearer_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-ambiguous-bearer")?;

    let mut server = Server::new();
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
