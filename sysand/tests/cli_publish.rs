// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! Every project in this file is written as a legacy `.project.json`
//! pair by hand rather than created through `sysand init`, which
//! writes a `model.toml`: what these fixtures pin is the json path,
//! which "TOML to KPAR" leaves unchanged.
//!
//! `publish_toml_project_with_a_stdlib_usage`, at the end of the file,
//! is the toml end-to-end case.
//!
//! Note `sysand publish` does *not* build: it uploads the archive
//! `sysand build` left in `output/`.

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
    let (temp_dir, cwd) = cli_init_json_project(None, "example-pub1", name, "1.0.0", Some("MIT"))?;
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
    run_sysand_ok(cwd, &["edit", "--metamodel", "sysml"], None)
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
    run_sysand_ok(cwd, &["edit", &format!("--{field}"), value], None)
}

fn bearer_env_for_url(url: &str) -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_TEST".to_owned(), format!("{url}/**"));
    env.insert(
        "SYSAND_CRED_TEST_BEARER_TOKEN".to_owned(),
        "test-token".to_owned(),
    );
    env
}

/// Register a `sysand-index-config.json` mock that tells the client the
/// `api_root` is at `<server>/api/`. The upload fixtures in this file
/// POST to `/api/v1/upload`, so the advertised `api_root` must carry
/// the `/api/` segment. Discovery is mandatory on first use; without
/// this mock the discovery fetch would 404 and, since a plain root
/// advertises no `api_root`, publish would refuse with "does not
/// advertise a publish endpoint" instead of reaching the upload mocks.
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
    mock_publish_with_bearer_times(server, token, 1)
}

/// The same, for a workspace publish, which uploads once per member.
fn mock_publish_with_bearer_times(server: &mut Server, token: &str, times: usize) -> mockito::Mock {
    server
        .mock("POST", "/api/v1/upload")
        .match_header("authorization", format!("Bearer {token}").as_str())
        .with_status(201)
        .with_body("created")
        .expect(times)
        .create()
}

/// A workspace publish resolves every member's archive before it
/// uploads anything, so an unbuildable member stops the command with
/// nothing sent.
#[test]
fn publish_workspace_with_an_unreadable_member_uploads_nothing() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["project1"])?;
    // Loads, and cannot be read as a project: `Workspace::load_with`
    // refuses a member that defines no project at all, which would fail
    // before the per-member resolution this test is about.
    std::fs::create_dir(cwd.join("project1"))?;
    std::fs::write(
        cwd.join("project1/model.toml"),
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"example-pub1\"\n\
         name = \"project1\"\nversion = \"1.0.0\"\nsources = [\"missing.sysml\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;

    let mut server = Server::new();
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 0);
    let env = bearer_env_for_url(server.url().as_str());

    run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .failure()
    // The failure names the member, which `KParBuildError` does not.
    .stderr(predicate::str::contains("`project1`"));
    upload_mock.assert();

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

/// A directory has no upload endpoint: the refusal names the write path
/// that does exist.
#[test]
fn publish_refuses_filesystem_index() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;
    std::fs::create_dir_all(cwd.join("mirror"))?;
    let out = run_sysand_in(&cwd, ["publish", "--index", "./mirror"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "publishing to a filesystem index is not supported",
        ))
        .stderr(predicate::str::contains("`sysand index add`"));
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
        "GITLAB_OIDC_TOKEN".to_owned(),
        "gitlab-oidc-token".to_owned(),
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
fn publish_trusted_publishing_requires_explicit_mode() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-trusted-space-value")?;
    let out = run_sysand_in(
        &cwd,
        [
            "publish",
            "--trusted-publishing",
            "--index",
            "http://localhost:1",
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "a value is required for '--trusted-publishing <MODE>'",
    ));

    Ok(())
}

#[test]
fn publish_trusted_publishing_always_uses_gitlab_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-trusted-always-gitlab")?;
    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let exchange_mock = mock_oidc_exchange(&mut server, "gitlab-oidc-token", "index-token");
    let publish_mock = mock_publish_with_bearer(&mut server, "index-token");

    let mut env = IndexMap::new();
    env.insert(
        "GITLAB_OIDC_TOKEN".to_owned(),
        "gitlab-oidc-token".to_owned(),
    );

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--trusted-publishing",
            "always",
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
fn publish_malformed_explicit_credentials_abort_before_trusted_publishing() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-malformed-explicit-creds")?;
    let mut env = IndexMap::new();
    env.insert(
        "SYSAND_CRED_TEST_BEARER_TOKEN".to_owned(),
        "test-token".to_owned(),
    );
    env.insert(
        "GITLAB_OIDC_TOKEN".to_owned(),
        "gitlab-oidc-token".to_owned(),
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
fn env_credentials_without_a_label_get_a_labels_are_required_error() -> TestResult {
    // Label-less names must not be misread as a pattern for a nonsense
    // group; the error explains the label grammar instead.
    let (_temp_dir, cwd) = setup_built_project("publish-label-less-creds")?;
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED".to_owned(), "https://acme.org".to_owned());
    env.insert("SYSAND_CRED_BASIC_USER".to_owned(), "user-a".to_owned());
    env.insert(
        "SYSAND_CRED_BASIC_PASS".to_owned(),
        "secret-pass-b".to_owned(),
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
            "SYSAND_CRED has no label; env credentials need one, as in SYSAND_CRED_MYINDEX",
        ))
        .stderr(predicate::str::contains("no matching authentication scheme").not())
        // Never any variable's value, in particular not the password.
        .stderr(predicate::str::contains("secret-pass-b").not())
        .stderr(predicate::str::contains("user-a").not());

    Ok(())
}

#[test]
fn env_credential_scheme_errors_never_print_the_variable_values() -> TestResult {
    // A pattern variable with no secret companion errors without echoing
    // its value: a misnamed variable can put a secret in any position.
    let (_temp_dir, cwd) = setup_built_project("publish-pattern-value-secrecy")?;
    let mut env = IndexMap::new();
    env.insert(
        "SYSAND_CRED_TEST".to_owned(),
        "accidental-secret-value".to_owned(),
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
            "SYSAND_CRED_TEST has no matching authentication scheme",
        ))
        .stderr(predicate::str::contains("accidental-secret-value").not());

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
            Matcher::Regex("multipart/form-data; boundary=.*".to_owned()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_owned()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="metadata""#.to_owned()),
            Matcher::Regex("Content-Type: application/json".to_owned()),
            Matcher::Regex(r#""kpar_sha256_digest":"[0-9a-f]{64}""#.to_owned()),
            Matcher::Regex(r#""normalized_publisher":"#.to_owned()),
            Matcher::Regex(r#""normalized_name":"#.to_owned()),
            Matcher::Regex(r#""version":"#.to_owned()),
            Matcher::Regex(r#""license":"#.to_owned()),
            Matcher::Regex(r#"name="kpar""#.to_owned()),
            Matcher::Regex("Content-Type: application/zip".to_owned()),
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
    // `IndexLocation` enforces the HTTP(S) invariant when the `--index`
    // argument is parsed, so a bad scheme is rejected before any network
    // work.
    let (_temp_dir, cwd) = setup_built_project_at("invalid-index", "artifact.kpar")?;
    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", "ftp://example.org"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "only `http` and `https` are supported",
        ))
        .stderr(predicate::str::contains("HTTP request failed").not());

    Ok(())
}

#[test]
fn publish_rejects_upload_endpoint_index_url() -> TestResult {
    // If the user pastes the full upload URL as the `--index` value,
    // discovery 404s against it and, since a plain root advertises no
    // `api_root`, the index has no API surface. Publish refuses before any
    // upload, pointing at the missing publish endpoint rather than trying
    // the paste.
    let (_temp_dir, cwd) = setup_built_project_at("upload-endpoint-index", "artifact.kpar")?;
    let mut server = Server::new();
    let config_mock = server
        .mock("GET", "/v1/upload/sysand-index-config.json")
        .with_status(404)
        .expect(1)
        .create();
    let publish_mock = server.mock("POST", "/v1/upload").expect(0).create();
    let endpoint_url = format!("{}/v1/upload", server.url());

    // No bearer credentials needed: publish bails on the missing API
    // surface before credential resolution.
    let out = run_sysand_in(
        &cwd,
        ["publish", "artifact.kpar", "--index", endpoint_url.as_str()],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "does not advertise a publish endpoint",
        ))
        .stderr(predicate::str::contains("HTTP request failed").not());
    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_rejects_invalid_semver_version() -> TestResult {
    // `sysand build` now validates the version itself, so a kpar with an
    // invalid version can no longer come from the normal build path; write
    // one directly to exercise `publish`'s own validation.
    let (_temp_dir, cwd) = init_project("invalid-version")?;

    let (kpar_bytes, _info, _meta) =
        build_index_kpar_bytes("acme", "invalid-version", "not-semver");
    std::fs::write(cwd.join("artifact.kpar"), kpar_bytes)?;

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
            Matcher::Regex("multipart/form-data; boundary=.*".to_owned()),
        )
        .match_header(
            "content-length",
            Matcher::Regex("^[1-9][0-9]{2,}$".to_owned()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="metadata""#.to_owned()),
            Matcher::Regex(r#""kpar_sha256_digest":"[0-9a-f]{64}""#.to_owned()),
            Matcher::Regex(r#""normalized_publisher":"acme-labs""#.to_owned()),
            Matcher::Regex(r#""normalized_name":"my.project-alpha""#.to_owned()),
            Matcher::Regex(r#""version":"1.0.0""#.to_owned()),
            Matcher::Regex(r#""license":"MIT""#.to_owned()),
            Matcher::Regex(r#"name="kpar""#.to_owned()),
            Matcher::Regex("Content-Type: application/zip".to_owned()),
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
    env.insert("SYSAND_CRED_TEST".to_owned(), pattern);
    env.insert("SYSAND_CRED_TEST_BASIC_USER".to_owned(), "user".to_owned());
    env.insert("SYSAND_CRED_TEST_BASIC_PASS".to_owned(), "pass".to_owned());

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
        // The CLI adds the `sysand auth login` remediation the core message
        // deliberately omits (core/CLI message split), naming the actual
        // index URL so the hint is copy-pasteable.
        .stderr(predicate::str::contains(format!(
            "run `sysand auth login {}",
            server.url()
        )))
        .stderr(predicate::str::contains("HTTP request failed").not());

    publish_mock.assert();
    config_mock.assert();

    Ok(())
}

#[test]
fn publish_uses_stored_credential_when_no_env_bearer_matches() -> TestResult {
    // A stored login (seeded through the debug-only file-backed store
    // seam, see cli_auth.rs) must reach the upload's Authorization header
    // when no `SYSAND_CRED_*` bearer matches: publish reads the store
    // directly, exactly once, during credential selection.
    let (_temp_dir, cwd) = setup_built_project("publish-stored-credential")?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = mock_publish_with_bearer(&mut server, "stored-tok");

    let store_dir = camino_tempfile::Utf8TempDir::with_prefix("sysand_cred_seam_")?;
    let store_path = store_dir.path().join("creds.json");
    // The blob as `sysand auth login` persists it, covering the whole
    // index host, so the derived glob matches the upload URL.
    fs::write(
        &store_path,
        format!(
            r#"{{"version":1,"credentials":[{{
                "key":"{url}/",
                "globs":["{url}/**"],
                "scheme":"bearer",
                "secret":"stored-tok",
                "validated":["publish"]}}]}}"#,
            url = server.url()
        ),
    )?;

    let mut env = IndexMap::new();
    env.insert(
        "SYSAND_TEST_CREDENTIAL_STORE".to_owned(),
        store_path.to_string(),
    );

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
fn publish_rejects_ambiguous_bearer_credentials() -> TestResult {
    let (_temp_dir, cwd) = setup_built_project("publish-ambiguous-bearer")?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server.mock("POST", "/api/v1/upload").expect(0).create();

    let base = server.url();
    let mut env = IndexMap::new();
    env.insert("SYSAND_CRED_A".to_owned(), format!("{base}/**"));
    env.insert(
        "SYSAND_CRED_A_BEARER_TOKEN".to_owned(),
        "token-a".to_owned(),
    );
    env.insert("SYSAND_CRED_B".to_owned(), format!("{base}/api/**"));
    env.insert(
        "SYSAND_CRED_B_BEARER_TOKEN".to_owned(),
        "token-b".to_owned(),
    );

    let out = run_sysand_in_with(&cwd, ["publish", "--index", base.as_str()], None, &env)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "multiple bearer token credentials from `SYSAND_CRED_*` environment variables configured for publish URL",
        ))
        // The error names every conflicting `SYSAND_CRED_<LABEL>` variable.
        .stderr(predicate::str::contains("SYSAND_CRED_A"))
        .stderr(predicate::str::contains("SYSAND_CRED_B"))
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
    // The bearer came from the `SYSAND_CRED_TEST` environment credential,
    // so the failure names that source with the env remediation
    // (design/credential-storage.md section 7).
    assert_publish_error_status(
        "publish-auth-401",
        401,
        "unauthorized",
        None,
        &[
            "authentication failed",
            "unauthorized",
            "came from `SYSAND_CRED_TEST`",
            "SYSAND_CRED_TEST_BEARER_TOKEN",
        ],
    )
}

#[test]
fn publish_403_maps_to_auth_error() -> TestResult {
    // 403 additionally points at `sysand auth status` (added by the CLI, per
    // the core/CLI message split: core states the condition, the frontend
    // names the command).
    assert_publish_error_status(
        "publish-auth-403",
        403,
        "forbidden",
        None,
        &[
            "authorization failed",
            "forbidden",
            "came from `SYSAND_CRED_TEST`",
            "run `sysand auth status`",
        ],
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

#[test]
fn publish_to_templated_index_without_api_root_reports_explicit_error() -> TestResult {
    // An index reached through a `{path}` URL template is file-serving
    // only; unless its discovery document sets `api_root`, publish must
    // fail with an actionable message before any upload is attempted.
    let (_temp_dir, cwd) = setup_built_project("test-publish")?;

    let mut server = Server::new();
    let config_mock = server
        .mock("GET", "/files/sysand-index-config.json/raw")
        .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
        .with_status(404)
        .expect(1)
        .create();

    let index = format!("{}/files/{{path}}/raw?ref=main", server.url());
    let out = run_sysand_in(&cwd, ["publish", "--index", index.as_str()], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "does not advertise a publish endpoint",
        ))
        .stderr(predicate::str::contains("index administrator"));

    config_mock.assert();
    Ok(())
}

#[test]
fn publish_index_with_unknown_placeholder_reports_parse_error() -> TestResult {
    // Template validation happens at argument parse time, before any
    // network or kpar work.
    let (_temp_dir, cwd) = init_project("test-publish")?;
    let out = run_sysand_in(
        &cwd,
        ["publish", "--index", "https://example.org/files/{file}/raw"],
        None,
    )?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains("unknown placeholder `{file}`"));

    Ok(())
}

/// `build` and `publish` agree about a standard-library usage.
///
/// The manifest is written by hand -- `setup_built_project` writes a
/// legacy json pair and runs `include --no-index-symbols`, which a
/// `model.toml` project refuses -- and it carries `license` and
/// `metamodel` because `publish` requires both.
///
/// The usage is the publisher-less `kpar-url` form, which
/// is what `publish`'s `check_std_libs` matches against -- the IRI, not
/// the publisher.
#[test]
fn publish_toml_project_with_a_stdlib_usage() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    fs::write(
        cwd.join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nlicense = \"MIT\"\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\nsources = [\"a.sysml\"]\n\n\
         [[inline-usages]]\nname = \"Kernel Semantic Library\"\n\
         kpar-url = \"https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar\"\n",
    )?;
    fs::write(cwd.join("a.sysml"), b"package A;\n")?;
    fs::create_dir(cwd.join("LICENSES"))?;
    fs::write(cwd.join("LICENSES/MIT.txt"), "MIT\n")?;

    // `publish` uploads what `build` left behind.
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = mock_publish_with_bearer(&mut server, "test-token");
    let env = bearer_env_for_url(server.url().as_str());

    run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .success();

    config_mock.assert();
    publish_mock.assert();
    Ok(())
}

// ------------------------ publishing a workspace in dependency order

/// A `model.toml` member at `at`, with a license file, a source, a
/// metamodel and optional `dir` usages of its siblings.
///
/// A *toml* member, because an inter-member `dir` usage is what creates
/// a dependency order, and `publish` refuses a typed path usage outright. A
/// toml build rewrites such a usage into `pkg:sysand/<pub>/<name>`
/// constrained to the target's version, which publishes; a
/// json build would keep it a path.
fn workspace_member(root: &Utf8Path, at: &str, deps: &[&str]) -> TestResult {
    let cwd = root.join(at);
    fs::create_dir_all(&cwd)?;
    let mut manifest = format!(
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"example-pub1\"\nname = \"{at}\"\n\
         version = \"1.0.0\"\nlicense = \"MIT\"\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\nsources = [\"test.sysml\"]\n"
    );
    for dep in deps {
        let name = dep.rsplit('/').next().unwrap_or(dep);
        manifest.push_str("\n[[inline-usages]]\npublisher = \"example-pub1\"\nname = \"");
        manifest.push_str(name);
        manifest.push_str("\"\ndir = \"");
        manifest.push_str(dep);
        manifest.push_str("\"\n");
    }
    fs::write(cwd.join("model.toml"), manifest)?;
    fs::write(cwd.join("test.sysml"), "package P;\n")?;
    fs::create_dir(cwd.join("LICENSES"))?;
    fs::File::create(cwd.join("LICENSES/MIT.txt"))?;
    Ok(())
}

fn workspace_of(root: &Utf8Path, members: &[&str]) -> TestResult {
    let entries: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
    fs::write(
        root.join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [{}]\n",
            entries.join(", ")
        ),
    )?;
    Ok(())
}

/// Every member is published, in dependency order. The order is read from
/// stderr's byte offsets: a mock records a hit count, and `contains` is
/// order-blind.
#[test]
fn publish_workspace_publishes_every_member() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["gamma", "beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &["../alpha"])?;
    workspace_member(&cwd, "gamma", &["../beta"])?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 3);
    let env = bearer_env_for_url(server.url().as_str());

    let out = run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?;
    out.clone().assert().success();
    // The config mock is `expect(1)`: discovery is a property of the
    // index, not of the archive, so it must not repeat per member.
    config_mock.assert();
    upload_mock.assert();

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let at = |name: &str| {
        stderr
            .find(&format!("Published example-pub1/{name}"))
            .unwrap_or_else(|| panic!("`{name}` was not reported as published:\n{stderr}"))
    };
    assert!(
        at("alpha") < at("beta") && at("beta") < at("gamma"),
        "members must publish dependency-first:\n{stderr}"
    );
    Ok(())
}

/// "a failure stops the sequence and reports what was published".
#[test]
fn publish_workspace_stops_at_the_first_failure() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["gamma", "beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &["../alpha"])?;
    workspace_member(&cwd, "gamma", &["../beta"])?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    // The 201 is created first, so it takes the first request; the
    // rejection then takes the second. A third upload would push it to
    // two hits and fail its assert.
    let ok_mock = mock_publish_with_bearer_times(&mut server, "test-token", 1);
    let reject_mock = server
        .mock("POST", "/api/v1/upload")
        .with_status(409)
        .with_body("conflict")
        .expect(1)
        .create();
    let env = bearer_env_for_url(server.url().as_str());

    run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .failure()
    .stderr(predicate::str::contains("already published"))
    .stderr(predicate::str::contains("example-pub1/alpha"));
    config_mock.assert();
    ok_mock.assert();
    reject_mock.assert();
    Ok(())
}

/// The archive exists and is not a zip. Unlike a *missing* archive,
/// which the existence check catches wherever it sits, this is only
/// found before the first upload if every payload is prepared up front.
#[test]
fn publish_workspace_with_a_malformed_archive_uploads_nothing() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &["../alpha"])?;
    run_sysand_ok(&cwd, &["build"], None)?;
    // `beta` comes second in dependency order.
    fs::write(cwd.join("output/beta-1.0.0.kpar"), b"not a zip\n")?;

    let mut server = Server::new();
    // The config mock matters: without it the command fails at
    // discovery, before any upload, and the test could not tell a
    // payload prepared up front from one prepared in the loop.
    let _config_mock = mock_index_config_api_at_api(&mut server);
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 0);
    let env = bearer_env_for_url(server.url().as_str());

    run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .failure();
    upload_mock.assert();
    Ok(())
}

/// From a member directory, only that member.
#[test]
fn publish_from_a_member_directory_publishes_only_that_member() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &[])?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 1);
    let env = bearer_env_for_url(server.url().as_str());

    run_sysand_in_with(
        &cwd.join("alpha"),
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .success()
    .stderr(predicate::str::contains("Published example-pub1/alpha"));
    config_mock.assert();
    upload_mock.assert();
    Ok(())
}

/// `--member <name>` from the root publishes exactly that member.
///
/// This is the `--member` selector of the `publish` row of
/// `workspaces.md`'s Command scopes table -- on the one command that
/// uploads, where publishing the wrong scope cannot be undone by
/// deleting a local artifact.
///
/// The negative assertions use the qualified `example-pub1/<name>`, not
/// the bare directory name: a member directory is called `alpha`, so a
/// bare `alpha` would also match a path in any future error line.
#[test]
fn publish_member_from_the_root_publishes_one() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["gamma", "beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &["../alpha"])?;
    workspace_member(&cwd, "gamma", &["../beta"])?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 1);
    let env = bearer_env_for_url(server.url().as_str());

    let out = run_sysand_in_with(
        &cwd,
        [
            "publish",
            "--member",
            "beta",
            "--index",
            server.url().as_str(),
        ],
        None,
        &env,
    )?;
    out.clone().assert().success();
    config_mock.assert();
    upload_mock.assert();

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("Published example-pub1/beta"),
        "`beta` must be the one published:\n{stderr}"
    );
    for other in ["example-pub1/alpha", "example-pub1/gamma"] {
        assert!(
            !stderr.contains(other),
            "`{other}` must not be published:\n{stderr}"
        );
    }
    Ok(())
}

/// `--workspace` from a member directory publishes every member, in
/// dependency order -- "forces all members", against the one the
/// working directory would otherwise imply.
#[test]
fn publish_workspace_from_a_member_dir_publishes_every_member() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    workspace_of(&cwd, &["gamma", "beta", "alpha"])?;
    workspace_member(&cwd, "alpha", &[])?;
    workspace_member(&cwd, "beta", &["../alpha"])?;
    workspace_member(&cwd, "gamma", &["../beta"])?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let upload_mock = mock_publish_with_bearer_times(&mut server, "test-token", 3);
    let env = bearer_env_for_url(server.url().as_str());

    let out = run_sysand_in_with(
        &cwd.join("beta"),
        ["publish", "--workspace", "--index", server.url().as_str()],
        None,
        &env,
    )?;
    out.clone().assert().success();
    config_mock.assert();
    upload_mock.assert();

    // `contains` is order-blind, so the order is the byte offsets.
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let at = |name: &str| {
        stderr
            .find(&format!("Published example-pub1/{name}"))
            .unwrap_or_else(|| panic!("`{name}` was not published:\n{stderr}"))
    };
    assert!(
        at("alpha") < at("beta") && at("beta") < at("gamma"),
        "members must publish dependency-first:\n{stderr}"
    );
    Ok(())
}

/// A built Syside KPAR passes publish's validation and is uploaded. The
/// manifest is written by hand.
#[test]
fn publish_accepts_a_built_syside_kpar() -> TestResult {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    fs::create_dir_all(cwd.join("LICENSES"))?;
    fs::write(
        cwd.join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = [\"main.sysml\"]\nlicense = \"MIT\"\n\
         metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"main.sysml\"\n",
    )?;
    fs::write(cwd.join("main.sysml"), "package Widgets;\n")?;
    fs::write(cwd.join("README.md"), "# widgets\n")?;
    fs::write(cwd.join("LICENSES/MIT.txt"), "MIT\n")?;
    run_sysand_ok(&cwd, &["build"], None)?;

    let mut server = Server::new();
    let config_mock = mock_index_config_api_at_api(&mut server);
    let publish_mock = server
        .mock("POST", "/api/v1/upload")
        .match_header("authorization", "Bearer test-token")
        .with_status(201)
        .with_body("created")
        .expect(1)
        .create();

    let env = bearer_env_for_url(server.url().as_str());
    run_sysand_in_with(
        &cwd,
        ["publish", "--index", server.url().as_str()],
        None,
        &env,
    )?
    .assert()
    .success();
    publish_mock.assert();
    config_mock.assert();
    Ok(())
}
