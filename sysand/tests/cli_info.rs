// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(feature = "alltests")]
use std::process::Command;

use std::io::Write as _;

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn info_basic_in_cwd() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out_init) =
        run_sysand(["init", "--version", "1.2.3", "--name", "info_basic"], None)?;
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

fn info_basic(use_iri: bool, use_auto: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out_new) = run_sysand(["new", "--version", "1.2.3", "info_basic"], None)?;
    out_new
        .assert()
        .success()
        .stdout(predicate::str::is_empty());

    fn add_iri_args<'a>(args: &mut Vec<&'a str>, use_auto: bool, path: &'a str) {
        if use_auto {
            args.push("--auto-location");
        } else {
            args.push("--iri");
        }
        args.push(path);
    }

    fn add_path_args<'a>(args: &mut Vec<&'a str>, use_auto: bool, path: &'a str) {
        if use_auto {
            args.push("--auto-location");
        } else {
            args.push("--path")
        }
        args.push(path);
    }

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
                add_iri_args(&mut args, use_auto, "file://info_basic");
            } else {
                add_path_args(&mut args, use_auto, "info_basic");
            }
            run_sysand_in(&cwd, args, None)?
        };

        out_relative
            .assert()
            .success()
            .stdout(predicate::str::contains("Name: info_basic"))
            .stdout(predicate::str::contains("Version: 1.2.3"));
    }

    let project_path: std::path::PathBuf = cwd.join("info_basic");
    let out_absolute = {
        let mut args = vec!["info"];
        if use_iri {
            let project_path_uri = url::Url::from_file_path(project_path).unwrap().to_string();
            add_iri_args(&mut args, use_auto, &project_path_uri);
            run_sysand_in(&cwd, args, None)?
        } else {
            let project_path_str = project_path.display().to_string();
            add_path_args(&mut args, use_auto, &project_path_str);
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
fn info_basic_path_explicit() -> Result<(), Box<dyn std::error::Error>> {
    info_basic(false, false)
}

#[test]
fn info_basic_path_auto() -> Result<(), Box<dyn std::error::Error>> {
    info_basic(false, true)
}

#[test]
fn info_basic_iri_explicit() -> Result<(), Box<dyn std::error::Error>> {
    info_basic(true, false)
}

#[test]
fn info_basic_iri_auto() -> Result<(), Box<dyn std::error::Error>> {
    info_basic(true, true)
}

#[test]
fn info_basic_http_url() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let git_mock = server
        .mock("GET", "/info/refs?service=git-upload-pack")
        .with_status(404)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let kpar_range_probe = server
        .mock("HEAD", "/")
        .with_status(404)
        .expect_at_most(1)
        .create();

    let kpar_download_try = server
        .mock("GET", "/")
        .with_status(404)
        .expect_at_most(1)
        .create();

    let info_mock_head = server
        .mock("HEAD", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3","usage":[]}"#)
        .create();

    let info_mock = server
        .mock("GET", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_http_url","version":"1.2.3","usage":[]}"#)
        .expect_at_most(3) // TODO: Reduce this to 1 after caching
        .create();

    let meta_mock_head = server
        .mock("HEAD", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect_at_most(3) // TODO: Reduce this to 1 after caching
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

// #[test]
// fn info_non_ranged_http_kpar() -> Result<(), Box<dyn std::error::Error>> {
//     let buf = {
//         let mut cursor = std::io::Cursor::new(vec![]);
//         let mut zip = zip::ZipWriter::new(&mut cursor);

//         let options = zip::write::SimpleFileOptions::default()
//             .compression_method(zip::CompressionMethod::Stored)
//             .unix_permissions(0o755);

//         zip.start_file("some_root_dir/.project.json", options)?;
//         zip.write_all(br#"{"name":"info_non_ranged_http_kpar","version":"1.2.3","usage":[]}"#)?;
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
fn info_basic_local_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempfile::TempDir::new()?;
    let zip_path = cwd.path().canonicalize()?.join("test.kpar");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file("some_root_dir/.project.json", options)?;
        zip.write_all(br#"{"name":"info_basic_local_kpar","version":"1.2.3","usage":[]}"#)?;
        zip.start_file("some_root_dir/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)?;

        zip.finish().unwrap();
    }

    let (_, _, out) = run_sysand(["info", "--path", &zip_path.to_string_lossy()], None)?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_local_kpar"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    Ok(())
}

#[cfg(feature = "alltests")]
#[test]
fn info_basic_file_git() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempfile::TempDir::new()?;

    {
        Command::new("git")
            .arg("init")
            .current_dir(cwd.path())
            .output()?
            .assert()
            .success();

        // TODO: Replace by commands::*::do_* when sufficiently complete, also use gix to create repo?
        std::fs::write(
            cwd.path().join(".project.json"),
            r#"{"name":"info_basic_file_git","version":"1.2.3","usage":[]}"#,
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
            .arg("commit")
            .arg("-m")
            .arg("test_commit")
            .current_dir(cwd.path())
            .output()?
            .assert()
            .success();
    }

    let (_, _, out) = run_sysand(
        [
            "info",
            "--uri",
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

#[test]
fn info_basic_index_url() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();

    let versions_mock = server
        .mock(
            "GET",
            "/e837859ce90bb1917c2698a6d62caa5786f67662fd1e35eb320f6e9da96939fe/versions.txt",
        )
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("1.2.3\n")
        .expect_at_most(1)
        .create();

    let project_mock_head = server
        .mock("HEAD", "/e837859ce90bb1917c2698a6d62caa5786f67662fd1e35eb320f6e9da96939fe/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_index_url","version":"1.2.3","usage":[]}"#)
        .expect_at_most(1)
        .create();

    let project_mock = server
        .mock("GET", "/e837859ce90bb1917c2698a6d62caa5786f67662fd1e35eb320f6e9da96939fe/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_basic_index_url","version":"1.2.3","usage":[]}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let meta_mock = server
        .mock("GET", "/e837859ce90bb1917c2698a6d62caa5786f67662fd1e35eb320f6e9da96939fe/1.2.3.kpar/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:info_basic_index_url",
            "--use-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_basic_index_url"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_mock.assert();
    project_mock_head.assert();
    project_mock.assert();
    meta_mock.assert();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:other",
            "--use-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project 'urn:kpar:other'",
    ));

    Ok(())
}

#[test]
fn info_multi_index_url() -> Result<(), Box<dyn std::error::Error>> {
    let mut server = mockito::Server::new();
    let mut server_alt = mockito::Server::new();

    let versions_mock = server
        .mock(
            "GET",
            "/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042/versions.txt",
        )
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("1.2.3\n")
        .expect_at_most(1)
        .create();

    let project_mock_head = server
        .mock("HEAD", "/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_multi_index_url","version":"1.2.3","usage":[]}"#)
        .expect_at_most(1)
        .create();

    let project_mock = server
        .mock("GET", "/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_multi_index_url","version":"1.2.3","usage":[]}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let meta_mock = server
        .mock("GET", "/f38ace6666fe279c9e856b2a25b14bf0a03b8c23ff1db524acf1afd78f66b042/1.2.3.kpar/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let versions_alt_mock = server_alt
        .mock(
            "GET",
            "/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565/versions.txt",
        )
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body("1.2.3\n")
        .expect_at_most(1)
        .create();

    let project_alt_mock_head = server_alt
        .mock("HEAD", "/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_multi_index_url_alt","version":"1.2.3","usage":[]}"#)
        .expect_at_most(1)
        .create();

    let project_alt_mock = server_alt
        .mock("GET", "/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565/1.2.3.kpar/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"info_multi_index_url_alt","version":"1.2.3","usage":[]}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let meta_alt_mock = server_alt
        .mock("GET", "/f0f4203b967855590901dc5c90f525d732015ca10598e333815cc30600874565/1.2.3.kpar/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .expect_at_most(2) // TODO: Reduce this to 1 after caching
        .create();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:info_multi_index_url",
            "--use-index",
            &server.url(),
            "--use-index",
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
            "--use-index",
            &server.url(),
            "--use-index",
            &server_alt.url(),
        ],
        None,
    )?;

    out.assert()
        .success()
        .stdout(predicate::str::contains("Name: info_multi_index_url_alt"))
        .stdout(predicate::str::contains("Version: 1.2.3"));

    versions_mock.assert();
    project_mock_head.assert();
    project_mock.assert();
    meta_mock.assert();

    versions_alt_mock.assert();
    project_alt_mock_head.assert();
    project_alt_mock.assert();
    meta_alt_mock.assert();

    let (_, _, out) = run_sysand(
        [
            "info",
            "--iri",
            "urn:kpar:other",
            "--use-index",
            &server.url(),
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project 'urn:kpar:other'",
    ));

    Ok(())
}

#[test]
fn info_detailed_verbs() -> Result<(), Box<dyn std::error::Error>> {
    let (_tmp, cwd, out) = run_sysand(["new", "info_detailed_verbs", "--version", "1.2.3"], None)?;
    out.assert().success();

    let project_path = &cwd.join("info_detailed_verbs");

    let get_field = |field: &'static str,
                     expected: Option<String>|
     -> Result<String, Box<dyn std::error::Error>> {
        let out = run_sysand_in(project_path, ["info", field], None)?;
        let stdout = out.stdout.clone();
        if let Some(expected) = expected {
            out.assert().success().stdout(expected);
        }
        Ok(String::from_utf8(stdout)?)
    };

    // Check that a field does/does not get cleared
    let try_clear =
        |field: &'static str, expected: bool| -> Result<(), Box<dyn std::error::Error>> {
            let before = get_field(field, None)?;

            let out = run_sysand_in(project_path, ["info", field, "--clear"], None)?;
            if expected {
                out.assert().success();
                get_field(field, Some("".to_string()))?;
            } else {
                out.assert()
                    .stderr(predicates::str::contains("unexpected argument"));
                get_field(field, Some(before))?;
            }
            Ok(())
        };

    let try_set = |field: &'static str,
                   value: &'static str,
                   expected: bool|
     -> Result<(), Box<dyn std::error::Error>> {
        let before = get_field(field, None)?;
        let out = run_sysand_in(project_path, ["info", field, "--set", value], None)?;
        if expected {
            out.assert().success();
            let mut expected_output = value.to_string();
            expected_output.push('\n');
            get_field(field, Some(expected_output))?;
        } else {
            out.assert()
                .failure()
                .stderr(predicates::str::contains("unexpected argument"));
            get_field(field, Some(before))?;
        }
        Ok(())
    };

    let try_add = |field: &'static str,
                   value: &'static str,
                   expected: bool|
     -> Result<(), Box<dyn std::error::Error>> {
        let before = get_field(field, None)?;
        let out = run_sysand_in(project_path, ["info", field, "--add", value], None)?;
        if expected {
            out.assert().success();
            let mut expected_output = before;
            expected_output.push_str(value);
            expected_output.push('\n');
            get_field(field, Some(expected_output))?;
        } else {
            out.assert()
                .failure()
                .stderr(predicates::str::contains("unexpected argument"));
            get_field(field, Some(before))?;
        }
        Ok(())
    };

    let try_remove = |field: &'static str,
                      index: &'static str,
                      expected: bool|
     -> Result<(), Box<dyn std::error::Error>> {
        let before = get_field(field, None)?;
        let out = run_sysand_in(project_path, ["info", field, "--remove", index], None)?;
        if expected {
            out.assert().success();
            let skipped = index.parse::<usize>()? - 1;
            let mut expected_output = "".to_string();
            for (i, line) in before.lines().enumerate() {
                if i != skipped {
                    expected_output.push_str(line);
                    expected_output.push('\n');
                }
            }
            get_field(field, Some(expected_output))?;
        } else {
            out.assert()
                .failure()
                .stderr(predicates::str::contains("unexpected argument"));
            get_field(field, Some(before))?;
        }
        Ok(())
    };

    get_field("name", Some("info_detailed_verbs\n".to_string()))?;
    try_set("name", "info_detailed_verbs_alt", true)?;
    try_clear("name", false)?;
    try_add("name", "name_1", false)?;
    try_remove("name", "1", false)?;
    get_field("version", Some("1.2.3\n".to_string()))?;
    try_set("version", "3.2.1", true)?;
    try_clear("version", false)?;
    try_add("version", "version_1", false)?;
    try_remove("version", "1", false)?;
    get_field("description", Some("".to_string()))?;
    try_set("description", "description", true)?;
    try_clear("description", true)?;
    try_add("description", "description_1", false)?;
    try_remove("description", "1", false)?;
    get_field("licence", Some("".to_string()))?;
    try_set("licence", "BSD4", true)?;
    try_clear("licence", true)?;
    try_add("licence", "licence_1", false)?;
    try_remove("licence", "1", false)?;
    get_field("license", Some("".to_string()))?;
    try_set("license", "BSD4", true)?;
    try_clear("license", true)?;
    try_add("license", "license_1", false)?;
    try_remove("license", "1", false)?;
    get_field("maintainer", Some("".to_string()))?;
    try_set("maintainer", "maintainer", true)?;
    try_clear("maintainer", true)?;
    try_add("maintainer", "maintainer_1", true)?;
    try_add("maintainer", "maintainer_2", true)?;
    try_add("maintainer", "maintainer_3", true)?;
    try_remove("maintainer", "2", true)?;
    get_field("website", Some("".to_string()))?;
    try_set("website", "www.example.com", true)?;
    try_clear("website", true)?;
    try_add("website", "website_1", false)?;
    try_remove("website", "1", false)?;
    get_field("topic", Some("".to_string()))?;
    try_set("topic", "example", true)?;
    try_clear("topic", true)?;
    try_add("topic", "topic_1", true)?;
    try_add("topic", "topic_2", true)?;
    try_add("topic", "topic_3", true)?;
    try_remove("topic", "2", true)?;
    get_field("usage", Some("".to_string()))?;
    try_set("usage", "usage", false)?;
    try_clear("usage", false)?;
    try_add("usage", "usage_1", false)?;
    try_remove("usage", "1", false)?;
    get_field("index", Some("".to_string()))?;
    try_set("index", "index", false)?;
    try_clear("index", false)?;
    try_add("index", "index_1", false)?;
    try_remove("index", "1", false)?;
    // get_field("created", Some("".to_string()))?;
    try_set("created", "created", false)?;
    try_clear("created", false)?;
    try_add("created", "created_1", false)?;
    try_remove("created", "1", false)?;
    get_field("metamodel", Some("".to_string()))?;
    try_set("metamodel", "metamodel", true)?;
    try_clear("metamodel", true)?;
    try_add("metamodel", "metamodel_1", false)?;
    try_remove("metamodel", "1", false)?;
    get_field("includes-derived", Some("".to_string()))?;
    try_set("includes-derived", "true", true)?;
    try_clear("includes-derived", true)?;
    try_add("includes-derived", "includes_1", false)?;
    try_remove("includes-derived", "1", false)?;
    get_field("includes-implied", Some("".to_string()))?;
    try_set("includes-implied", "false", true)?;
    try_clear("includes-implied", true)?;
    try_add("includes-implied", "includes_1", false)?;
    try_remove("includes-implied", "1", false)?;
    get_field("checksum", Some("".to_string()))?;
    try_set("checksum", "checksum", false)?;
    try_clear("checksum", false)?;
    try_add("checksum", "checksum_1", false)?;
    try_remove("checksum", "1", false)?;

    Ok(())
}
