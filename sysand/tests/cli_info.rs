// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "alltests")]
use std::process::Command;

use std::{error::Error, io::Write as _};

use assert_cmd::prelude::*;
use camino::Utf8PathBuf;
use camino_tempfile::Utf8TempDir;
use indexmap::IndexMap;
use mockito::Matcher;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;
use sysand_core::{project::utils::wrapfs, utils::sha256_lowercase_hex};

/// Register a `sysand-index-config.json` 404 mock on `server`.
/// Configured index URLs go through the discovery step, which fetches this
/// URL first. These tests don't exercise the discovery-document path; they
/// just want the client to proceed with `index_root` defaulting to the
/// discovery root (these reads need only `index_root`; `api_root` stays
/// unset).
fn mock_index_config_absent(server: &mut mockito::Server, expected_count: usize) -> mockito::Mock {
    server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(expected_count)
        .create()
}

#[test]
fn info_basic_in_cwd() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out_init) = cli_init_project_basic("a", "info_basic", "1.2.3")?;
    out_init
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    let out = run_sysand_in(&cwd, ["info"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    Ok(())
}

#[test]
fn info_prints_all_usage_types() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out_init) =
        cli_init_project_basic("pub1", "info_all_usage_types", "1.2.3")?;
    out_init.assert().success();

    // One usage of each shape: legacy `resource` (with and without a version
    // constraint), `dir`, and `kpar_path`.
    std::fs::write(
        cwd.join(".project.json"),
        r#"{
  "name": "info_all_usage_types",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:constrained_dep",
      "versionConstraint": "^1.0.0"
    },
    {
      "resource": "urn:kpar:unconstrained_dep"
    },
    {
      "dir": "deps/dir_dep",
      "publisher": "acme",
      "name": "dir_dep"
    },
    {
      "kparPath": "deps/kpar_dep.kpar",
      "publisher": "acme",
      "name": "kpar_dep"
    }
  ]
}
"#,
    )?;

    let out = run_sysand_in(&cwd, ["info"], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Usages:"))
        .stdout(predicate::str::contains(
            "IRI `urn:kpar:constrained_dep` (^1.0.0)",
        ))
        .stdout(predicate::str::contains("IRI `urn:kpar:unconstrained_dep`"))
        .stdout(predicate::str::contains(
            "`acme/dir_dep` from `deps/dir_dep`",
        ))
        .stdout(predicate::str::contains(
            "`acme/kpar_dep` in `deps/kpar_dep.kpar`",
        ));

    Ok(())
}

fn info_basic(use_iri: bool) -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out_init) =
        cli_init_project(Some("info_basic"), "a", None, Some("1.2.3"), None)?;
    out_init
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    if !use_iri {
        // FIXME: Relative file IRIs are currently not supported because
        // according to https://datatracker.ietf.org/doc/html/rfc8089:
        //
        // > The path component represents the absolute path to the file in the
        // > file system.
        //
        // We could potentially allow relative references here
        // (https://www.rfc-editor.org/rfc/rfc3986#section-4.2). However, this
        // decision would effectively relax the requirement in the KerML 10.3
        // saying that the project is identified by IRI and we need to have a
        // deeper discussion about this.
        let out_relative = {
            let mut args = vec!["info"];
            if use_iri {
                args.extend(["--iri", "file://info_basic"]);
            } else {
                args.extend(["--dir", "info_basic"]);
            }
            run_sysand_in(&cwd, args, None)?
        };

        out_relative
            .assert()
            .success()
            .stdout(predicate::str::contains("Name: info_basic"))
            .stdout(predicate::str::contains("Version: 1.2.3"));
    }

    let project_path: Utf8PathBuf = cwd.join("info_basic");
    let out_absolute = {
        let mut args = vec!["info"];
        #[expect(clippy::branches_sharing_code, reason = "does not compile otherwise")]
        if use_iri {
            let project_path_uri = url::Url::from_file_path(project_path).unwrap().to_string();
            args.extend(["--iri", &project_path_uri]);
            run_sysand_in(&cwd, args, None)?
        } else {
            args.extend(["--dir", project_path.as_str()]);
            run_sysand_in(&cwd, args, None)?
        }
    };

    out_absolute
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    Ok(())
}

#[test]
fn info_basic_path_explicit() -> Result<(), Box<dyn Error>> {
    info_basic(false)
}

#[test]
fn info_basic_iri_explicit() -> Result<(), Box<dyn Error>> {
    info_basic(true)
}

/// The positional argument is a `<publisher>/<name>` identifier, never an
/// IRI or a path
#[test]
fn info_positional_is_identifier() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, _) = cli_init_project(Some("info_positional"), "acme", None, None, None)?;

    run_sysand_in(&cwd, ["info", "acme/some-project"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "describing by `<publisher>/<name>` identifier is not supported yet",
        ));

    for invalid_identifier in [
        "name",
        "urn:kpar:test",
        "info_positional",
        "c:/foo",
        "a/b/c",
    ] {
        run_sysand_in(&cwd, ["info", invalid_identifier], None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains(format!(
                "invalid value '{invalid_identifier}' for '[IDENTIFIER]'"
            )))
            .stderr(predicate::str::contains(
                "`--dir`, `--kpar-path` or `--iri`",
            ))
            .stderr(predicate::str::contains("`--get <FIELD>`"));
    }

    Ok(())
}

#[test]
fn info_dir_and_kpar_path_are_not_interchangeable() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, _) = cli_init_project(Some("info_kinds"), "acme", None, None, None)?;
    let project = cwd.join("info_kinds");
    let file = project.join(".project.json");

    run_sysand_in(&cwd, ["info", "--kpar-path", project.as_str()], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a file"))
        .stderr(predicate::str::contains("use `--dir`"));

    run_sysand_in(&cwd, ["info", "--dir", file.as_str()], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a directory"))
        .stderr(predicate::str::contains("use `--kpar-path`"));

    Ok(())
}

#[test]
fn info_basic_http_url_noauth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();

    let git_mock = server
        .mock("GET", "/info/refs?service=git-upload-pack")
        .with_status(404)
        .expect(1)
        .create();

    let kpar_range_probe = server.mock("HEAD", "/").with_status(404).expect(0).create();

    // One call expected: the resolver tries the URL as a kpar once while
    // resolving it. It is not tried again for the local-cache match, since
    // with nothing installed there is nothing the candidate could match.
    // Pin the count; further reductions would be a resolver-level change.
    let kpar_download_try = server.mock("GET", "/").with_status(404).expect(1).create();

    let info_mock_head = server
        .mock("HEAD", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1)
        .create();

    let info_mock = server
        .mock("GET", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_head = server
        .mock("HEAD", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let (_, _, out) = run_sysand(["info", "--iri", &server.url()], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_http_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    git_mock.assert();

    info_mock_head.assert();
    meta_mock_head.assert();

    kpar_range_probe.assert();
    kpar_download_try.assert();

    info_mock.assert();
    meta_mock.assert();

    Ok(())
}

#[test]
fn info_basic_http_url_irrelevant_auth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();

    let git_mock = server
        .mock("GET", "/info/refs?service=git-upload-pack")
        .with_status(404)
        .expect(1)
        .create();

    let kpar_range_probe = server.mock("HEAD", "/").with_status(404).expect(0).create();

    let kpar_download_try = server
        .mock("GET", "/")
        .with_status(404)
        // See the matching comment in `info_basic_http_url_noauth`.
        .expect(1)
        .create();

    let info_mock_head = server
        .mock("HEAD", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1)
        .create();

    let info_mock = server
        .mock("GET", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_head = server
        .mock("HEAD", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let (_, _, out) = run_sysand_with(
        ["info", "--iri", &server.url()],
        None,
        &IndexMap::from([
            ("SYSAND_CRED_TEST", "http://irrelevant.example.com:*/**"),
            ("SYSAND_CRED_TEST_BASIC_USER", "user_1234"),
            ("SYSAND_CRED_TEST_BASIC_PASS", "pass_4321"),
        ]),
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_http_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    git_mock.assert();

    info_mock_head.assert();
    meta_mock_head.assert();

    kpar_range_probe.assert();
    kpar_download_try.assert();

    info_mock.assert();
    meta_mock.assert();

    Ok(())
}

#[test]
fn info_basic_http_url_auth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();

    let git_mock = server
        .mock("GET", "/info/refs?service=git-upload-pack")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .expect(1)
        .create();

    // let kpar_range_probe = server
    //     .mock("HEAD", "/")
    //     .match_header("authorization", Matcher::Missing)
    //     .with_status(404)
    //     .expect(1)
    //     .create();

    let kpar_download_try = server
        .mock("GET", "/")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        // See the matching comment in `info_basic_http_url_noauth`.
        .expect(1)
        .create();

    let kpar_download_try_auth = server
        .mock("GET", "/")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let info_mock_head = server
        .mock("HEAD", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1) // TODO: Reduce this
        .create();

    let info_mock_head_auth = server
        .mock("HEAD", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1) // TODO: Reduce this
        .create();

    let info_mock = server
        .mock("GET", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let info_mock_auth = server
        .mock("GET", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_head = server
        .mock("HEAD", "/.meta.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock_head_auth = server
        .mock("HEAD", "/.meta.json")
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(404)
        .match_header("authorization", Matcher::Missing)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_auth = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .match_header(
            "authorization",
            Matcher::Exact("Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==".to_owned()),
        )
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let (_, _, out) = run_sysand_with(
        ["info", "--iri", &server.url()],
        None,
        &IndexMap::from([
            ("SYSAND_CRED_TEST", "http://127.0.0.1:*/**"),
            ("SYSAND_CRED_TEST_BASIC_USER", "user_1234"),
            ("SYSAND_CRED_TEST_BASIC_PASS", "pass_4321"),
        ]),
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_http_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    git_mock.assert();

    info_mock_head.assert();
    info_mock_head_auth.assert();
    meta_mock_head.assert();
    meta_mock_head_auth.assert();

    // kpar_range_probe.assert();
    kpar_download_try.assert();
    kpar_download_try_auth.assert();

    info_mock.assert();
    info_mock_auth.assert();

    meta_mock.assert();
    meta_mock_auth.assert();

    Ok(())
}

#[test]
fn info_bearer_http_url_auth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();

    let git_mock = server
        .mock("GET", "/info/refs?service=git-upload-pack")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .expect(1)
        .create();

    // let kpar_range_probe = server
    //     .mock("HEAD", "/")
    //     .match_header("authorization", Matcher::Missing)
    //     .with_status(404)
    //     .expect(1)
    //     .create();

    let kpar_download_try = server
        .mock("GET", "/")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        // See the matching comment in `info_basic_http_url_noauth`.
        .expect(1)
        .create();

    let kpar_download_try_auth = server
        .mock("GET", "/")
        .match_header(
            "authorization",
            Matcher::Exact("Bearer this_is_a_token".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let info_mock_head = server
        .mock("HEAD", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1) // TODO: Reduce this
        .create();

    let info_mock_head_auth = server
        .mock("HEAD", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Bearer this_is_a_token".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(1) // TODO: Reduce this
        .create();

    let info_mock = server
        .mock("GET", "/.project.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let info_mock_auth = server
        .mock("GET", "/.project.json")
        .match_header(
            "authorization",
            Matcher::Exact("Bearer this_is_a_token".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_head = server
        .mock("HEAD", "/.meta.json")
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock_head_auth = server
        .mock("HEAD", "/.meta.json")
        .match_header(
            "authorization",
            Matcher::Exact("Bearer this_is_a_token".to_owned()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(404)
        .match_header("authorization", Matcher::Missing)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let meta_mock_auth = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .match_header(
            "authorization",
            Matcher::Exact("Bearer this_is_a_token".to_owned()),
        )
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect(2) // TODO: Reduce this to 1
        .create();

    let (_, _, out) = run_sysand_with(
        ["info", "--iri", &server.url()],
        None,
        &IndexMap::from([
            ("SYSAND_CRED_TEST", "http://127.0.0.1:*/**"),
            ("SYSAND_CRED_TEST_BEARER_TOKEN", "this_is_a_token"),
        ]),
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_http_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    git_mock.assert();

    info_mock_head.assert();
    info_mock_head_auth.assert();
    meta_mock_head.assert();
    meta_mock_head_auth.assert();

    // kpar_range_probe.assert();
    kpar_download_try.assert();
    kpar_download_try_auth.assert();

    info_mock.assert();
    info_mock_auth.assert();

    meta_mock.assert();
    meta_mock_auth.assert();

    Ok(())
}

// #[test]
// fn info_non_ranged_http_kpar() -> Result<(), Box<dyn std::error::Error>> {
//     let buf = {
//         let mut cursor = std::io::Cursor::new(vec![]);
//         let mut zip = zip::ZipWriter::new(&mut cursor);

//         let options = zip::write::SimpleFileOptions::default()
//             .compression_method(zip::CompressionMethod::Stored)
//             .unix_permissions(0o755);

//         zip.start_file("some_root_dir/.project.json", options)?;
//         zip.write_all(br#"{"name":"info_non_ranged_http_kpar","version":"1.2.3"}"#)?;
//         zip.start_file("some_root_dir/.meta.json", options)?;
//         zip.write_all(br#"{"index":{},"created":"123"}"#)?;
//         zip.start_file("some_root_dir/test.sysml", options)?;
//         zip.write_all(br#"package Test;"#)?;

//         zip.finish().unwrap();

//         cursor.flush()?;
//         cursor.into_inner()
//     };

//     let mut server = mockito::Server::new();

//     let kpar_probe = server
//         .mock("HEAD", "/info_non_ranged_http_kpar.kpar")
//         .with_status(200)
//         .with_header("content-type", "application/zip")
//         .with_body(&buf)
//         .create();

//     let get_kpar = server
//         .mock("GET", "/info_non_ranged_http_kpar.kpar")
//         .with_status(200)
//         .with_header("content-type", "application/zip")
//         .with_body(&buf)
//         .create();

//     let url = format!("{}/info_non_ranged_http_kpar.kpar", server.url());

//     let (_, _, out) = run_sysand(["info", "--iri", &url], None)?;

//     out.assert()
//         .success()
//         .stdout(predicate::str::contains("Name: info_non_ranged_http_kpar"))
//         .stdout(predicate::str::contains("Version: 1.2.3"));

//     kpar_probe.assert();
//     get_kpar.assert();

//     Ok(())
// }

#[test]
fn info_basic_local_kpar() -> Result<(), Box<dyn Error>> {
    let cwd = Utf8TempDir::new()?;
    let zip_path = wrapfs::canonicalize(cwd.path())?.join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file("some_root_dir/.project.json", options)?;
        zip.write_all(br#"{"name":"info_basic_local_kpar","version":"1.2.3"}"#)?;
        zip.start_file("some_root_dir/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)?;

        zip.finish().unwrap();
    }

    let (_, _, out) = run_sysand(["info", "--kpar-path", zip_path.as_str()], None)?;
    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_local_kpar"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    Ok(())
}

#[cfg(feature = "alltests")]
#[test]
fn info_basic_file_git() -> Result<(), Box<dyn Error>> {
    let cwd = Utf8TempDir::new()?;

    {
        git_init(cwd.path())?;

        // TODO: Replace by commands::*::do_* when sufficiently complete, also use gix to create repo?
        std::fs::write(
            cwd.path().join(".project.json"),
            r#"{"name":"info_basic_file_git","version":"1.2.3"}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".project.json")
            .current_dir(cwd.path())
            .output()?
            .assert()
            .success();

        std::fs::write(
            cwd.path().join(".meta.json"),
            r#"{"index":{},"created":"123"}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".meta.json")
            .current_dir(cwd.path())
            .output()?
            .assert()
            .success();

        // std::fs::write(cwd.path().join("test.sysml"), "package Test;")?;
        // Command::new("git")
        //     .arg("add")
        //     .arg("test.sysml")
        //     .current_dir(cwd.path())
        //     .output()?
        //     .assert()
        //     .success();

        Command::new("git")
            .args(["commit", "-m", "test_commit"])
            .current_dir(cwd.path())
            .output()?
            .assert()
            .success();
    }

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            url::Url::from_file_path(cwd.path()).unwrap().as_str(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_file_git"))
        .stdout(predicate::str::contains("Version: 1.2.3"));
    Ok(())
}

/// Render a minimal `.project.json` body for the given name/version.
fn project_json_for(name: &str, version: &str) -> String {
    format!(r#"{{"name":"{name}","version":"{version}"}}"#)
}

/// Render a minimal `.meta.json` body. The fixed timestamp keeps any test
/// that hashes the body reproducible.
const TEST_META_JSON_BODY: &str = r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#;

/// Build a `versions.json` body advertising a single entry. The kpar digest is a
/// placeholder because `sysand info` reads the
/// per-version JSON directly and never downloads the archive.
fn versions_json_for(version: &str) -> String {
    versions_json_body(&[versions_json_entry_body(
        version,
        42,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )])
}

#[test]
fn info_basic_index_url() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 2);

    let iri_dir = "/_iri/e837859ce90bb1917c2698a6d62caa5786f67662fd1e35eb320f6e9da96939fe";

    let project_body = project_json_for("info_basic_index_url", "1.2.3");
    let versions_body = versions_json_for("1.2.3");

    let versions_mock = server
        .mock("GET", format!("{iri_dir}/versions.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_body)
        .expect(1)
        .create();

    let project_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.project.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_body)
        .expect(1)
        .create();

    let meta_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.meta.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    // `info` reads per-version JSON directly; the kpar endpoint must not fire.
    let kpar_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:info_basic_index_url",
            "--default-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_index_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();

    // Catch-all for any _iri/<hash>/versions.json we didn't explicitly mock —
    // return 404 so the resolver treats the IRI as absent from this index.
    let missing_versions_mock = server
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:other",
            "--default-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "IRI `urn:kpar:other` was not found: no resolver was able to resolve the project",
    ));
    config_mock.assert();
    missing_versions_mock.assert();

    Ok(())
}

/// Register `versions.json` plus the per-version `.project.json`/`.meta.json`
/// pair for every version of a project served under `project_dir`. `info`
/// reads the per-version JSON of *each* candidate before it picks one, so all
/// of them have to be mocked
fn mock_index_project(
    server: &mut mockito::Server,
    project_dir: &str,
    name: &str,
    versions: &[&str],
) -> Vec<mockito::Mock> {
    let versions_body = versions_json_body(
        &versions
            .iter()
            .map(|version| {
                versions_json_entry_body(
                    version,
                    42,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
            })
            .collect::<Vec<_>>(),
    );

    let mut mocks = vec![
        server
            .mock("GET", format!("{project_dir}/versions.json").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&versions_body)
            .expect(1)
            .create(),
    ];

    for version in versions {
        mocks.push(
            server
                .mock(
                    "GET",
                    format!("{project_dir}/{version}/.project.json").as_str(),
                )
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(project_json_for(name, version))
                .create(),
        );
        mocks.push(
            server
                .mock(
                    "GET",
                    format!("{project_dir}/{version}/.meta.json").as_str(),
                )
                .with_status(200)
                .with_header("content-type", "application/json")
                .with_body(TEST_META_JSON_BODY)
                .create(),
        );
    }

    mocks
}

/// `info` must always describe the latest version, including pre-releases
#[test]
fn info_index_iri_takes_the_highest_version_including_a_prerelease() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 1);

    let iri = "urn:kpar:info_index_prerelease";
    let iri_dir = format!("/_iri/{}", sha256_lowercase_hex(iri));
    let mocks = mock_index_project(
        &mut server,
        &iri_dir,
        "info_index_prerelease",
        &["2.0.0-beta.1", "1.0.0"],
    );

    let (_, _, out) = run_sysand(
        ["info", "--iri", iri, "--default-index", &server.url()],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_index_prerelease"))
        .stdout(predicate::str::contains("Version: 2.0.0-beta.1"));

    config_mock.assert();
    mocks[0].assert();

    Ok(())
}

/// `info` must always describe the latest version, including pre-releases
#[test]
fn info_index_purl_takes_the_highest_version_including_a_prerelease() -> Result<(), Box<dyn Error>>
{
    let mut server = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 1);

    let mocks = mock_index_project(
        &mut server,
        "/acme/widget",
        "widget",
        &["3.0.0-beta.1", "2.0.0"],
    );

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "pkg:sysand/acme/widget",
            "--default-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: widget"))
        .stdout(predicate::str::contains("Version: 3.0.0-beta.1"));

    config_mock.assert();
    mocks[0].assert();

    Ok(())
}

#[test]
fn info_multi_index_url_noauth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();
    let mut server_alt = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 3);
    let config_mock_alt = mock_index_config_absent(&mut server_alt, 2);

    let iri_dir = "/_iri/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042";
    let iri_dir_alt = "/_iri/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565";

    let project_body = project_json_for("info_multi_index_url", "1.2.3");
    let project_alt_body = project_json_for("info_multi_index_url_alt", "1.2.3");
    let versions_body = versions_json_for("1.2.3");
    let versions_alt_body = versions_json_for("1.2.3");

    let versions_mock = server
        .mock("GET", format!("{iri_dir}/versions.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_body)
        .expect(1)
        .create();

    let project_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.project.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_body)
        .expect(1)
        .create();

    let meta_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.meta.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    // `info` reads per-version JSON directly; the kpar endpoint must not fire.
    let kpar_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    let versions_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/versions.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_alt_body)
        .expect(1)
        .create();

    let project_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.project.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_alt_body)
        .expect(1)
        .create();

    let meta_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.meta.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    let kpar_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    // 404 for IRIs not explicitly mocked: required for the cross-index miss
    // (server hit for `_alt`, server_alt hit for the primary) and for the
    // final "urn:kpar:other" lookup below.
    let server_missing_mock = server
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(2)
        .create();
    let server_alt_missing_mock = server_alt
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:info_multi_index_url",
            "--index",
            &server.url(),
            "--default-index",
            &server_alt.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_multi_index_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:info_multi_index_url_alt",
            "--index",
            &server.url(),
            "--default-index",
            &server_alt.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_multi_index_url_alt"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();
    versions_alt_mock.assert();
    project_json_alt_mock.assert();
    meta_json_alt_mock.assert();
    kpar_alt_mock.assert();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:other",
            "--default-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "IRI `urn:kpar:other` was not found: no resolver was able to resolve the project",
    ));
    config_mock.assert();
    config_mock_alt.assert();
    server_missing_mock.assert();
    server_alt_missing_mock.assert();

    Ok(())
}

#[test]
fn info_multi_index_url_auth() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();
    let mut server_alt = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 6);
    let config_mock_alt = mock_index_config_absent(&mut server_alt, 2);

    let iri_dir = "/_iri/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042";
    let iri_dir_alt = "/_iri/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565";
    let basic_auth = "Basic dXNlcl8xMjM0OnBhc3NfNDMyMQ==";

    let project_body = project_json_for("info_multi_index_url", "1.2.3");
    let project_alt_body = project_json_for("info_multi_index_url_alt", "1.2.3");
    let versions_body = versions_json_for("1.2.3");
    let versions_alt_body = versions_json_for("1.2.3");

    let versions_mock = server
        .mock("GET", format!("{iri_dir}/versions.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .expect(1)
        .create();

    let versions_mock_auth = server
        .mock("GET", format!("{iri_dir}/versions.json").as_str())
        .match_header("authorization", Matcher::Exact(basic_auth.to_owned()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_body)
        .expect(1)
        .create();

    // Unauthenticated 404 lets the auth policy retry with credentials —
    // matches the same retry-on-404 pattern as the versions.json pair above.
    let project_json_mock_404 = server
        .mock("GET", format!("{iri_dir}/1.2.3/.project.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .expect(1)
        .create();

    let project_json_mock_auth = server
        .mock("GET", format!("{iri_dir}/1.2.3/.project.json").as_str())
        .match_header("authorization", Matcher::Exact(basic_auth.to_owned()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_body)
        .expect(1)
        .create();

    let meta_json_mock_404 = server
        .mock("GET", format!("{iri_dir}/1.2.3/.meta.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(404)
        .expect(1)
        .create();

    let meta_json_mock_auth = server
        .mock("GET", format!("{iri_dir}/1.2.3/.meta.json").as_str())
        .match_header("authorization", Matcher::Exact(basic_auth.to_owned()))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    // expect(0): info reads `.project.json` / `.meta.json` directly and never
    // hits the kpar endpoint, in either auth branch.
    let kpar_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    let versions_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/versions.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_alt_body)
        .expect(1)
        .create();

    let project_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.project.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_alt_body)
        .expect(1)
        .create();

    let meta_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.meta.json").as_str())
        .match_header("authorization", Matcher::Missing)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    let kpar_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    // 404 for any other IRI's versions.json, on both servers.
    let server_missing_mock = server
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(4)
        .create();
    let server_alt_missing_mock = server_alt
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let server_pattern = format!("http://{}/**", server.host_with_port());
    let auth_env = IndexMap::from([
        ("SYSAND_CRED_TEST", server_pattern.as_ref()),
        ("SYSAND_CRED_TEST_BASIC_USER", "user_1234"),
        ("SYSAND_CRED_TEST_BASIC_PASS", "pass_4321"),
    ]);

    let (_, _, out) = run_sysand_with(
        [
            "info",
            "--iri",
            "urn:kpar:info_multi_index_url",
            "--index",
            &server.url(),
            "--default-index",
            &server_alt.url(),
        ],
        None,
        &auth_env,
    )?;

    versions_mock.assert();
    versions_mock_auth.assert();
    project_json_mock_404.assert();
    project_json_mock_auth.assert();
    meta_json_mock_404.assert();
    meta_json_mock_auth.assert();
    kpar_mock.assert();

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_multi_index_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let (_, _, out) = run_sysand_with(
        [
            "info",
            "--iri",
            "urn:kpar:info_multi_index_url_alt",
            "--index",
            &server.url(),
            "--default-index",
            &server_alt.url(),
        ],
        None,
        &auth_env,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_multi_index_url_alt"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_alt_mock.assert();
    project_json_alt_mock.assert();
    meta_json_alt_mock.assert();
    kpar_alt_mock.assert();

    let (_, _, out) = run_sysand_with(
        [
            "info",
            "--iri",
            "urn:kpar:other",
            "--default-index",
            &server.url(),
        ],
        None,
        &auth_env,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "IRI `urn:kpar:other` was not found: no resolver was able to resolve the project",
    ));
    config_mock.assert();
    config_mock_alt.assert();
    server_missing_mock.assert();
    server_alt_missing_mock.assert();

    Ok(())
}

#[test]
fn info_multi_index_url_config() -> Result<(), Box<dyn Error>> {
    let mut server = mockito::Server::new();
    let mut server_alt = mockito::Server::new();
    let config_mock = mock_index_config_absent(&mut server, 2);
    let config_mock_alt = mock_index_config_absent(&mut server_alt, 2);

    let iri_dir = "/_iri/1206faf209922d2c3c3ce220d5b78b6001b1ec42ab1304d65590d1749453c5b5";
    let iri_dir_alt = "/_iri/bf1998eaeb56282e6dd62686b1ea51dfeffded6964aa53cc89cf70a9a2627c97";

    let project_body = project_json_for("info_multi_index_url_config", "1.2.3");
    let project_alt_body = project_json_for("info_multi_index_url_config_alt", "1.2.3");
    let versions_body = versions_json_for("1.2.3");
    let versions_alt_body = versions_json_for("1.2.3");

    let versions_mock = server
        .mock("GET", format!("{iri_dir}/versions.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_body)
        .expect(1)
        .create();

    let project_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.project.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_body)
        .expect(1)
        .create();

    let meta_json_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/.meta.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    // expect(0): info reads `.project.json` / `.meta.json` and never hits
    // the kpar endpoint.
    let kpar_mock = server
        .mock("GET", format!("{iri_dir}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    let versions_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/versions.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&versions_alt_body)
        .expect(1)
        .create();

    let project_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.project.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(&project_alt_body)
        .expect(1)
        .create();

    let meta_json_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/.meta.json").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(TEST_META_JSON_BODY)
        .expect(1)
        .create();

    let kpar_alt_mock = server_alt
        .mock("GET", format!("{iri_dir_alt}/1.2.3/project.kpar").as_str())
        .expect(0)
        .create();

    let server_missing_mock = server
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();
    let server_alt_missing_mock = server_alt
        .mock(
            "GET",
            Matcher::Regex(r"^/_iri/[a-f0-9]+/versions\.json$".to_owned()),
        )
        .with_status(404)
        .expect(1)
        .create();

    let (_temp_dir, cwd) = new_temp_cwd()?;

    let cfg = format!(
        r#"
    [[index]]
    url = "{}"

    [[index]]
    url = "{}"
    default = true
    "#,
        server.url(),
        server_alt.url()
    );

    let cfg_path = cwd.join(sysand_core::config::local_fs::CONFIG_FILE);
    std::fs::write(&cfg_path, cfg)?;

    let (_, _, out) = run_sysand(
        ["info", "--iri", "urn:kpar:info_multi_index_url_config"],
        Some(cfg_path.as_str()),
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "Name: info_multi_index_url_config",
        ))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    let (_, _, out) = run_sysand(
        ["info", "--iri", "urn:kpar:info_multi_index_url_config_alt"],
        Some(cfg_path.as_str()),
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains(
            "Name: info_multi_index_url_config_alt",
        ))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_mock.assert();
    project_json_mock.assert();
    meta_json_mock.assert();
    kpar_mock.assert();
    versions_alt_mock.assert();
    project_json_alt_mock.assert();
    meta_json_alt_mock.assert();
    kpar_alt_mock.assert();
    config_mock.assert();
    config_mock_alt.assert();
    server_missing_mock.assert();
    server_alt_missing_mock.assert();

    Ok(())
}

#[test]
fn info_get_and_edit_fields() -> Result<(), Box<dyn Error>> {
    let (_tmp, cwd, out) = cli_init_project(Some("info_fields"), "a", None, Some("1.2.3"), None)?;
    out.assert().success();
    let project_path = &cwd.join("info_fields");

    let get = |field: &str| -> Result<String, Box<dyn Error>> {
        let out = run_sysand_in(project_path, ["info", "--get", field], None)?;
        let stdout = out.assert().success().get_output().stdout.clone();
        Ok(String::from_utf8(stdout)?)
    };
    let edit = |args: &[&str]| -> Result<(), Box<dyn Error>> {
        run_sysand_in(
            project_path,
            std::iter::once("edit").chain(args.iter().copied()),
            None,
        )?
        .assert()
        .success();
        Ok(())
    };

    assert_eq!(get("name")?, "info_fields\n");
    edit(&["--name", "info_fields_alt"])?;
    assert_eq!(get("name")?, "info_fields_alt\n");

    assert_eq!(get("publisher")?, "a\n");
    edit(&["--publisher", "a_alt"])?;
    assert_eq!(get("publisher")?, "a_alt\n");

    assert_eq!(get("version")?, "1.2.3\n");
    edit(&["--version", "3.2.1"])?;
    assert_eq!(get("version")?, "3.2.1\n");

    assert_eq!(get("description")?, "");
    edit(&["--description", "description"])?;
    assert_eq!(get("description")?, "description\n");
    edit(&["--clear-description"])?;
    assert_eq!(get("description")?, "");

    for (set, clear) in [
        ("--license", "--clear-license"),
        ("--licence", "--clear-licence"),
    ] {
        edit(&[set, "BSD-4-Clause"])?;
        assert_eq!(get("license")?, "BSD-4-Clause\n");
        assert_eq!(get("licence")?, "BSD-4-Clause\n");
        edit(&[clear])?;
        assert_eq!(get("license")?, "");
    }

    for (value, expected) in [
        ("www.example.com", "https://www.example.com\n"),
        ("http://www.example.com", "http://www.example.com\n"),
        ("https://www.example.com", "https://www.example.com\n"),
    ] {
        edit(&["--website", value])?;
        assert_eq!(get("website")?, expected);
    }
    edit(&["--clear-website"])?;
    assert_eq!(get("website")?, "");

    for (field, add, remove, clear) in [
        (
            "maintainer",
            "--add-maintainer",
            "--remove-maintainer",
            "--clear-maintainers",
        ),
        ("topic", "--add-topic", "--remove-topic", "--clear-topics"),
    ] {
        assert_eq!(get(field)?, "");
        edit(&[add, "x1", add, "x2", add, "x3"])?;
        assert_eq!(get(field)?, "x1\nx2\nx3\n");
        edit(&[remove, "x2"])?;
        assert_eq!(get(field)?, "x1\nx3\n");
        // Clearing is applied before adding, replacing the list
        edit(&[clear, add, "y"])?;
        assert_eq!(get(field)?, "y\n");
        // Removing a value that is not there fails and changes nothing
        run_sysand_in(project_path, ["edit", remove, "nope"], None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains(format!(
                "project has no {field} `nope`"
            )));
        assert_eq!(get(field)?, "y\n");
        edit(&[clear])?;
        assert_eq!(get(field)?, "");
    }

    for (field, value) in [("includes-derived", "true"), ("includes-implied", "false")] {
        assert_eq!(get(field)?, "");
        edit(&[&format!("--{field}"), value])?;
        assert_eq!(get(field)?, format!("{value}\n"));
        edit(&[&format!("--clear-{field}")])?;
        assert_eq!(get(field)?, "");
    }

    // Several fields at once, across `.project.json` and `.meta.json`
    edit(&[
        "--description",
        "d",
        "--add-topic",
        "t",
        "--includes-implied",
        "true",
    ])?;
    assert_eq!(get("description")?, "d\n");
    assert_eq!(get("topic")?, "t\n");
    assert_eq!(get("includes-implied")?, "true\n");

    // Fields that are not edited directly
    for field in ["usage", "index", "checksum", "metamodel"] {
        assert_eq!(get(field)?, "", "field: {field}");
    }
    assert!(!get("created")?.is_empty());
    for flag in ["--usage", "--index", "--created", "--checksum"] {
        run_sysand_in(project_path, ["edit", flag, "x"], None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument"));
    }

    // A mistyped field is rejected by clap
    run_sysand_in(project_path, ["info", "--get", "naem"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "invalid value 'naem' for '--get <FIELD>'",
        ));

    Ok(())
}

/// `sysand edit` alone prints its help; with other arguments but no edits
/// it fails
#[test]
fn edit_requires_an_edit() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "edit_nothing", "1.2.3")?;
    out.assert().success();

    // Not through `run_sysand_in`, which appends `--no-config`
    std::process::Command::new(assert_cmd::cargo::cargo_bin!("sysand"))
        .arg("edit")
        .current_dir(&cwd)
        .env("NO_COLOR", "1")
        .assert()
        .failure()
        // The program name differs between platforms (e.g. `sysand.exe`)
        .stderr(predicate::str::is_match(r"(?m)^Usage: \S+ edit ")?);
    for args in [&["edit"][..], &["edit", "--dir", "."]] {
        run_sysand_in(&cwd, args.iter().copied(), None)?
            .assert()
            .failure()
            .stderr(predicate::str::contains("no edits given"));
    }

    Ok(())
}

/// `sysand edit --dir` edits the project in the given directory
#[test]
fn edit_dir() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out) = cli_init_project(Some("edit_dir"), "a", None, None, None)?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["edit", "--dir", "edit_dir", "--version", "2.0.0"],
        None,
    )?
    .assert()
    .success();
    run_sysand_in(
        &cwd,
        ["info", "--dir", "edit_dir", "--get", "version"],
        None,
    )?
    .assert()
    .success()
    .stdout("2.0.0\n");

    run_sysand_in(
        &cwd,
        [
            "edit",
            "--dir",
            "edit_dir/.project.json",
            "--version",
            "3.0.0",
        ],
        None,
    )?
    .assert()
    .failure()
    .stderr(predicate::str::contains("is not a directory"));

    Ok(())
}

#[test]
fn edit_metamodel() -> Result<(), Box<dyn Error>> {
    let (_tmp, cwd, out) = cli_init_project(
        Some("info_custom_metamodel"),
        "a",
        None,
        Some("1.2.3"),
        None,
    )?;
    out.assert().success();

    let project_path = &cwd.join("info_custom_metamodel");

    let get_metamodel = |expected: Option<String>| -> Result<String, Box<dyn Error>> {
        let out = run_sysand_in(project_path, ["info", "--get", "metamodel"], None)?;
        let stdout = out.stdout.clone();
        if let Some(v) = expected {
            out.assert().success().stdout(v);
        }
        Ok(String::from_utf8(stdout)?)
    };

    let try_set = |flags_values: &[&str],
                   expected_value_err: Result<&str, &str>|
     -> Result<(), Box<dyn Error>> {
        let before = get_metamodel(None)?;
        let out = run_sysand_in(
            project_path,
            std::iter::once("edit").chain(flags_values.iter().copied()),
            None,
        )?;
        match expected_value_err {
            Ok(v) => {
                out.assert().success();
                let mut expected_output = v.to_owned();
                expected_output.push('\n');
                get_metamodel(Some(expected_output))?;
            }
            Err(e) => {
                out.assert().failure().stderr(predicates::str::contains(e));
                get_metamodel(Some(before))?;
            }
        }
        Ok(())
    };

    // Default release
    try_set(
        &["--metamodel", "sysml"],
        Ok("https://www.omg.org/spec/SysML/20250201"),
    )?;
    try_set(
        &["--metamodel", "kerml"],
        Ok("https://www.omg.org/spec/KerML/20250201"),
    )?;
    // Explicitly specified release
    try_set(
        &["--metamodel", "sysml", "--metamodel-release", "20250201"],
        Ok("https://www.omg.org/spec/SysML/20250201"),
    )?;
    try_set(
        &["--metamodel", "kerml", "--metamodel-release", "20250201"],
        Ok("https://www.omg.org/spec/KerML/20250201"),
    )?;
    // Unknown release
    try_set(
        &["--metamodel", "sysml", "--metamodel-release", "20230201"],
        Err("invalid value '20230201'"),
    )?;
    // Custom release
    try_set(
        &["--metamodel", "sysml", "--metamodel-release-custom", "123"],
        Ok("https://www.omg.org/spec/SysML/123"),
    )?;
    try_set(
        &["--metamodel", "kerml", "--metamodel-release-custom", "456"],
        Ok("https://www.omg.org/spec/KerML/456"),
    )?;
    // Invalid custom release
    try_set(
        &["--metamodel", "kerml", "--metamodel-release-custom", "abc"],
        Err("invalid value 'abc' for '--metamodel-release-custom"),
    )?;
    // Custom metamodel
    try_set(&["--custom-metamodel", "mm1"], Ok("mm1"))?;
    // Release without a metamodel
    try_set(
        &["--metamodel-release", "20250201"],
        Err("the following required arguments were not provided"),
    )?;

    // Flag conflicts
    try_set(
        &["--metamodel", "kerml", "--custom-metamodel", "abc123"],
        Err("the argument '--metamodel <KIND>' cannot be used with '--custom-metamodel"),
    )?;
    try_set(
        &[
            "--metamodel",
            "kerml",
            "--metamodel-release",
            "20250201",
            "--metamodel-release-custom",
            "123",
        ],
        Err(
            "the argument '--metamodel-release <YYYYMMXX>' cannot be used with '--metamodel-release-custom",
        ),
    )?;
    try_set(
        &[
            "--custom-metamodel",
            "abc123",
            "--metamodel-release-custom",
            "123",
        ],
        Err(
            "the argument '--custom-metamodel <METAMODEL>' cannot be used with '--metamodel-release-custom <YYYYMMXX>'",
        ),
    )?;
    try_set(
        &[
            "--custom-metamodel",
            "abc123",
            "--metamodel-release",
            "20250201",
        ],
        Err(
            "the argument '--custom-metamodel <METAMODEL>' cannot be used with '--metamodel-release <YYYYMMXX>'",
        ),
    )?;

    run_sysand_in(project_path, ["edit", "--clear-metamodel"], None)?
        .assert()
        .success();
    // An unset field prints nothing, not even an empty line
    get_metamodel(Some(String::new()))?;

    Ok(())
}

/// `sysand edit --name` and `--publisher` should reject invalid values and
/// leave `.project.json` unchanged
#[test]
fn edit_rejects_invalid_name_and_publisher() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "info_set_invalid", "1.2.3")?;
    out.assert().success();
    let original = std::fs::read_to_string(cwd.join(".project.json"))?;

    for (field, value_name, value, msg) in [
        ("name", "NAME", "", "name cannot be empty"),
        ("name", "NAME", "a/b", "name cannot contain `/`"),
        ("name", "NAME", "a:b", "name cannot contain `:`"),
        ("name", "NAME", "a\tb", "name cannot contain `\\t`"),
        ("publisher", "PUBLISHER", "", "publisher cannot be empty"),
        (
            "publisher",
            "PUBLISHER",
            "a/b",
            "publisher cannot contain `/`",
        ),
        (
            "publisher",
            "PUBLISHER",
            "a:b",
            "publisher cannot contain `:`",
        ),
        (
            "publisher",
            "PUBLISHER",
            "a\nb",
            "publisher cannot contain `\\n`",
        ),
    ] {
        let out = run_sysand_in(&cwd, ["edit", &format!("--{field}"), value], None)?;

        out.assert().failure().stderr(
            predicate::str::contains(format!(
                "invalid value '{value}' for '--{field} <{value_name}>'"
            ))
            .and(predicate::str::contains(msg)),
        );
        assert_eq!(
            std::fs::read_to_string(cwd.join(".project.json"))?,
            original,
            "field: {field}, value: {value:?}"
        );
    }

    Ok(())
}

/// `sysand edit --name` and `--publisher` should accept values containing
/// spaces
#[test]
fn edit_accepts_spaces_in_name_and_publisher() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "info_set_spaces", "1.2.3")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["edit", "--name", "My Project", "--publisher", "Acme Labs"],
        None,
    )?
    .assert()
    .success();

    assert_eq!(
        std::fs::read_to_string(cwd.join(".project.json"))?,
        r#"{
  "name": "My Project",
  "publisher": "Acme Labs",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}
