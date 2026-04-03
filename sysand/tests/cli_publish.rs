// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

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
    let (temp_dir, cwd, out) = run_sysand(["init", "--version", "1.0.0", "--name", name], None)?;
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
fn test_publish_missing_kpar() -> TestResult {
    let (_temp_dir, cwd) = init_project("test-publish")?;
    let out = run_sysand_in(&cwd, ["publish"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("kpar file not found"))
        .stderr(predicate::str::contains("sysand build"));

    Ok(())
}

#[test]
fn test_publish_explicit_missing_kpar() -> TestResult {
    let (_temp_dir, cwd) = init_project("test-publish")?;
    let out = run_sysand_in(&cwd, ["publish", "nonexistent.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("kpar file not found"));

    Ok(())
}

#[test]
fn test_publish_network_error() -> TestResult {
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
fn test_publish_uses_config_default_index() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;

    let mut server = Server::new();
    let publish_mock = server
        .mock("POST", "/api/v1/upload")
        .match_header(
            "content-type",
            Matcher::Regex("multipart/form-data; boundary=.*".to_string()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_string()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="purl""#.to_string()),
            Matcher::Regex(r#"name="file""#.to_string()),
            Matcher::Regex(r#"filename=".*\.kpar""#.to_string()),
            Matcher::Regex(r#"Content-Type: application/octet-stream"#.to_string()),
        ]))
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create();

    let config_path = cwd.join("publish-test.toml");
    std::fs::write(
        &config_path,
        format!("[[index]]\nurl = \"{}\"\ndefault = true\n", server.url()),
    )?;

    let env = bearer_env_for_url(server.url().as_str());
    let out = run_sysand_in_with(&cwd, ["publish"], Some(config_path.as_str()), &env)?;
    out.assert().success();
    publish_mock.assert();

    Ok(())
}

#[test]
fn test_publish_prefers_configured_default_index() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;

    let mut non_default_server = Server::new();
    let non_default_publish_mock = non_default_server
        .mock("POST", "/api/v1/upload")
        .expect(0)
        .create();

    let mut default_server = Server::new();
    let default_publish_mock = default_server
        .mock("POST", "/api/v1/upload")
        .match_header(
            "content-type",
            Matcher::Regex("multipart/form-data; boundary=.*".to_string()),
        )
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create();

    let config_path = cwd.join("publish-test.toml");
    std::fs::write(
        &config_path,
        format!(
            "[[index]]\nurl = \"{}\"\n[[index]]\nurl = \"{}\"\ndefault = true\n",
            non_default_server.url(),
            default_server.url()
        ),
    )?;

    let env = bearer_env_for_url(default_server.url().as_str());
    let out = run_sysand_in_with(&cwd, ["publish"], Some(config_path.as_str()), &env)?;
    out.assert().success();
    default_publish_mock.assert();
    non_default_publish_mock.assert();

    Ok(())
}

#[test]
fn test_publish_explicit_path_outside_project_dir() -> TestResult {
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
fn test_publish_invalid_index_url_errors_early() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project_at("invalid-index", "artifact.kpar")?;
    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "not-a-url"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("invalid index URL"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn test_publish_rejects_invalid_semver_version() -> TestResult {
    let (_temp_dir, cwd) = init_project("invalid-version")?;

    let project_file = cwd.join(".project.json");
    let project_json = std::fs::read_to_string(&project_file)?;
    let project_json =
        project_json.replace("\"version\": \"1.0.0\"", "\"version\": \"not-semver\"");
    std::fs::write(project_file, project_json)?;

    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("version field"))
        .stderr(predicate::str::contains("Semantic Versioning 2.0 version"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn test_publish_rejects_noncanonicalizable_publisher() -> TestResult {
    let (_temp_dir, cwd) = init_project("valid-publish-name")?;
    set_project_field(&cwd, "publisher", "bad__publisher")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("publisher field"))
        .stderr(predicate::str::contains("must be 3-50 characters"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn test_publish_rejects_noncanonicalizable_name() -> TestResult {
    let (_temp_dir, cwd) = init_project("valid-publish-name")?;
    set_project_field(&cwd, "name", "bad__name")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "http://localhost:1"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("name field"))
        .stderr(predicate::str::contains("must be 3-50 characters"))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn test_publish_canonicalizes_modern_project_id() -> TestResult {
    let (_temp_dir, cwd) = init_project("seed-project")?;
    set_project_field(&cwd, "publisher", "Acme Labs")?;
    set_project_field(&cwd, "name", "My.Project Alpha")?;
    include_basic_model(&cwd)?;
    build_kpar_at(&cwd, "artifact.kpar")?;

    let mut server = Server::new();
    let publish_mock = server
        .mock("POST", "/api/v1/upload")
        .match_header(
            "content-type",
            Matcher::Regex("multipart/form-data; boundary=.*".to_string()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_string()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="purl""#.to_string()),
            Matcher::Regex("pkg:sysand/acme-labs/my\\.project-alpha@1\\.0\\.0".to_string()),
            Matcher::Regex(r#"name="file""#.to_string()),
            Matcher::Regex(r#"filename="artifact\.kpar""#.to_string()),
            Matcher::Regex(r#"Content-Type: application/octet-stream"#.to_string()),
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
fn test_publish_ignores_basic_auth_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-basic-auth-ignored")?;

    let mut server = Server::new();
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let config_path = cwd.join("publish-test.toml");
    std::fs::write(
        &config_path,
        format!("[[index]]\nurl = \"{}\"\ndefault = true\n", server.url()),
    )?;

    let pattern = format!("{}/**", server.url());
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_TEST", pattern.as_str());
    env.insert("SYSAND_CRED_TEST_BASIC_USER", "user");
    env.insert("SYSAND_CRED_TEST_BASIC_PASS", "pass");

    let out = run_sysand_in_with(&cwd, ["publish"], Some(config_path.as_str()), &env)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "no bearer token credentials configured for publish URL",
        ))
        .stderr(predicate::str::contains("HTTP request failed").not());

    publish_mock.assert();

    Ok(())
}
