// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! `sysand add` and `sysand remove`, in both manifest formats.
//!
//! Tests of `--iri` and `--iri-path` usages, which only a `.project.json`
//! can declare, create their project with `sysand init --kind project-json`.
//! Tests of typed usages (`--dir`, `--kpar-path`, `<publisher>/<name>`) run
//! over [`KINDS`]. Dependency projects whose format is irrelevant are made
//! with the default kind, except those installed through `--as-local-src`: a
//! `model.toml` project has no checksum and can only be editable. The tests of
//! `model.toml` text are at the end of the file.

use std::fs;

use assert_cmd::prelude::*;
use mockito::Server;
use predicates::prelude::{predicate::str::contains, *};
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH},
    lock::{Lock, Lockfile},
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// The manifest kinds `sysand init --kind` writes
const KINDS: [&str; 2] = ["project-json", "model-toml"];

/// A project of the default kind in a fresh temporary directory
fn new_project(
    publisher: &str,
    name: &str,
    version: &str,
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd, out) = cli_init_project_basic(publisher, name, version)?;
    out.assert().success();
    Ok((temp_dir, cwd))
}

/// A project of the default kind in the existing directory `dir`
fn init_project_in(
    dir: &camino::Utf8Path,
    publisher: &str,
    name: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    cli_init_project_in(dir, None, publisher, Some(name), Some(version), None)?
        .assert()
        .success();
    Ok(())
}

/// A project of manifest kind `kind` in a fresh temporary directory
fn new_project_of_kind(
    kind: &str,
    publisher: &str,
    name: &str,
    version: &str,
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let (temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--kind",
            kind,
            "--metamodel",
            SYSML,
            "--publisher",
            publisher,
            "--name",
            name,
            "--version",
            version,
        ],
        None,
    )?
    .assert()
    .success();
    Ok((temp_dir, cwd))
}

/// The usages the project at `cwd` declares, as `sysand info --get usage`
/// prints them
fn declared_usages(cwd: &camino::Utf8Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let out = run_sysand_in(cwd, ["info", "--get", "usage"], None)?;
    out.clone().assert().success();
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// The text of the manifest of the project at `cwd`, whatever its kind
fn manifest_text(cwd: &camino::Utf8Path) -> Result<String, Box<dyn std::error::Error>> {
    let name = if exists(cwd.join("model.toml")) {
        "model.toml"
    } else {
        ".project.json"
    };
    Ok(fs::read_to_string(cwd.join(name))?)
}

/// For a `.project.json` project, asserts that its manifest holds an index
/// usage of `publisher`/`name` with `constraint`, as json
fn assert_json_index_usage(
    kind: &str,
    cwd: &camino::Utf8Path,
    publisher: &str,
    name: &str,
    constraint: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if kind == "project-json" {
        let manifest = manifest_text(cwd)?;
        assert!(
            manifest.contains(&format!(
                r#"{{
      "publisher": "{publisher}",
      "name": "{name}",
      "versionConstraint": "{constraint}"
    }}"#
            )),
            "{manifest}"
        );
    }
    Ok(())
}

#[test]
fn add_and_remove_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "echo", "add-and-remove", "1.2.3", None)?;

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--iri", "urn:kpar:test"], None)?;

    out.assert()
        .success()
        .stderr(contains("Adding usage: IRI `urn:kpar:test`"));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove",
  "publisher": "echo",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:test"], None)?;

    out.assert().success().stderr(contains(
        "Removing `urn:kpar:test` from usages
     Removed `urn:kpar:test`",
    ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove",
  "publisher": "echo",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_rejects_sysand_shorthand() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "foxtrot", "reject-add-shorthand", "1.2.3", None)?;

    for shorthand in ["acme-labs/my.project", "Acme Labs/My.Project"] {
        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--iri", shorthand], None)?;

        out.assert()
            .failure()
            .stderr(contains("for '--iri <IRI>'"))
            .stderr(contains(
                "use `--dir`, `--kpar-path` or `--iri-path` instead",
            ))
            .stderr(contains("pass it without `--iri`"));
    }

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "reject-add-shorthand",
  "publisher": "foxtrot",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_path_like_iri_suggests_path_options() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["add", "--iri", "acme/bravo/c"], None)?;

    out.assert().failure().stderr(contains(
        "use `--dir`, `--kpar-path` or `--iri-path` instead",
    ));

    Ok(())
}

#[test]
fn remove_rejects_sysand_shorthand() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "hotel", "reject-remove-shorthand", "1.2.3", None)?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "pkg:sysand/acme-labs/my.project",
        ],
        None,
    )?
    .assert()
    .success();

    for shorthand in ["acme-labs/my.project", "Acme Labs/My.Project"] {
        let out = run_sysand_in(&cwd, ["remove", "--iri", shorthand], None)?;

        out.assert()
            .failure()
            .stderr(contains("for '--iri <IRI>'"))
            .stderr(contains("use `--iri-path` instead"))
            .stderr(contains("pass it without `--iri`"));
    }

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "reject-remove-shorthand",
  "publisher": "hotel",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "pkg:sysand/acme-labs/my.project"
    }
  ]
}
"#
    );

    Ok(())
}

#[test]
fn remove_path_like_iri_suggests_iri_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["remove", "--iri", "acme/bravo/c"], None)?;

    out.assert()
        .failure()
        .stderr(contains("use `--iri-path` instead"));

    Ok(())
}

/// Add and remove usages with `--iri-path <path>`
#[test]
fn add_and_remove_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1) =
        cli_init_json_project(None, "india", "add-and-remove-path1", "1.2.3", None)?;
    let (_temp_dir2, cwd2) =
        cli_init_json_project(None, "india", "add-and-remove-path2", "1.2.3", None)?;
    let file_url = file_url_from_path(&cwd2);

    let out = run_sysand_in(
        &cwd1,
        ["add", "--no-lock", "--iri-path", cwd2.as_str()],
        None,
    )?;

    out.assert()
        .success()
        .stderr(contains(format!("Adding usage: IRI `{file_url}`")));

    let info_json = fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add-and-remove-path1",
  "publisher": "india",
  "version": "1.2.3",
  "usage": [
    {{
      "resource": "{file_url}"
    }}
  ]
}}
"#
        )
    );

    let out = run_sysand_in(&cwd1, ["remove", "--iri-path", cwd2.as_str()], None)?;

    out.assert().success().stderr(contains(format!(
        "Removing `{file_url}` from usages
     Removed `{file_url}`"
    )));

    let info_json = fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove-path1",
  "publisher": "india",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn config_file_must_be_readable() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "juliet",
        "config-file-must-be-readable",
        "1.2.3",
        None,
    )?;

    // Missing, or a directory. Only `add` with a source creates it.
    let missing = cwd.join("missing.toml");
    for config_path in [missing.as_str(), cwd.as_str()] {
        for args in [
            &["lock"][..],
            &["add", "--no-lock", "--iri", "urn:kpar:test"],
        ] {
            let out = run_sysand_in(&cwd, args.iter().copied(), Some(config_path))?;
            out.assert()
                .failure()
                .stderr(contains(format!("failed to read file\n  `{config_path}`")));
        }
    }
    assert!(!exists(&missing));

    Ok(())
}

#[test]
fn add_and_remove_as_editable() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "juliet",
        "--as-editable",
        "local/test",
        r#"editable = "local/test""#,
    )
}

#[test]
fn add_and_remove_as_local_src() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "kilo",
        "--as-local-src",
        "local/test",
        r#"src_path = "local/test""#,
    )
}

#[test]
fn add_and_remove_as_local_kpar() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "lima",
        "--as-local-kpar",
        "local/test.kpar",
        r#"kpar_path = "local/test.kpar""#,
    )
}

/// A source path is written resolved, however it is spelled.
#[test]
fn a_source_path_is_written_resolved() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "mike",
        "--as-local-kpar",
        "local/../aaa/./test.kpar",
        r#"kpar_path = "aaa/test.kpar""#,
    )
}

#[test]
fn add_and_remove_as_remote_src() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "mike",
        "--as-remote-src",
        "https://www.example.com/test",
        r#"remote_src = "https://www.example.com/test""#,
    )
}

#[test]
fn add_and_remove_as_remote_kpar() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "november",
        "--as-remote-kpar",
        "https://www.example.com/test.kpar",
        r#"remote_kpar = "https://www.example.com/test.kpar""#,
    )
}

#[test]
fn add_and_remove_as_remote_git() -> Result<(), Box<dyn std::error::Error>> {
    check_add_and_remove_with_source(
        "november",
        "--as-remote-git",
        "https://www.example.com/test.git",
        r#"remote_git = "https://www.example.com/test.git""#,
    )
}

fn check_add_and_remove_with_source(
    publisher: &str,
    flag: &str,
    value: &str,
    expected_source: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, publisher, "add-and-remove", "1.2.3", None)?;
    // A local source must exist: its path is canonicalized.
    if flag == "--as-editable" || flag == "--as-local-src" {
        fs::create_dir_all(cwd.join(value))?;
    } else if flag == "--as-local-kpar" {
        let kpar = cwd.join(value);
        fs::create_dir_all(kpar.parent().expect("a file in a directory"))?;
        fs::write(kpar, "")?;
    }

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--iri", "urn:kpar:test", flag, value],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "  Adding usage: IRI `urn:kpar:test`
    Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add-and-remove",
  "publisher": "{publisher}",
  "version": "1.2.3",
  "usage": [
    {{
      "resource": "urn:kpar:test"
    }}
  ]
}}
"#
        )
    );

    let config = fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        format!(
            r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    {{ {expected_source} }},
]
"#
        )
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:test"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:test` from usages
     Removed `urn:kpar:test`
    Creating env
     Syncing env
             nothing to do: env is already up to date
    Removing source for `urn:kpar:test` from configuration file at `{config_path}`
    Removing empty configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add-and-remove",
  "publisher": "{publisher}",
  "version": "1.2.3"
}}
"#
        )
    );

    assert!(!is_file(&config_path));

    Ok(())
}

#[test]
fn add_without_locking_keeps_the_manifest_when_the_source_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "oscar", "add-missing", "1.2.3", None)?;
    let config_path = cwd.join("sysand.toml");
    let before = fs::read_to_string(cwd.join(".project.json"))?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:missing",
            "--from-path",
            "missing",
        ],
        Some(config_path.as_str()),
    )?
    .assert()
    .failure();

    assert_eq!(fs::read_to_string(cwd.join(".project.json"))?, before);
    assert!(!is_file(&config_path));

    Ok(())
}

#[test]
fn add_and_remove_from_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "oscar", "add-and-remove", "1.2.3", None)?;

    let config_path = cwd.join("sysand.toml");

    fs::create_dir_all(cwd.join("local/test"))?;

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:test-src",
            "--from-path",
            "local/test",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "  Adding usage: IRI `urn:kpar:test-src`
    Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test-src` to configuration file at `{config_path}`"
    )));

    fs::File::create_new(cwd.join("local/test.kpar"))?;

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:test-kpar",
            "--from-path",
            "local/test.kpar",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Adding usage: IRI `urn:kpar:test-kpar`
      Adding source for `urn:kpar:test-kpar` to configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove",
  "publisher": "oscar",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test-src"
    },
    {
      "resource": "urn:kpar:test-kpar"
    }
  ]
}
"#
    );

    let config = fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test-src",
]
sources = [
    { editable = "local/test" },
]

[[project]]
identifiers = [
    "urn:kpar:test-kpar",
]
sources = [
    { kpar_path = "local/test.kpar" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:test-src", "--no-lock"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:test-src` from usages
     Removed `urn:kpar:test-src`
    Removing source for `urn:kpar:test-src` from configuration file at `{config_path}`"
    )));

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:test-kpar", "--no-lock"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:test-kpar` from usages
     Removed `urn:kpar:test-kpar`
    Removing source for `urn:kpar:test-kpar` from configuration file at `{config_path}`
    Removing empty configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove",
  "publisher": "oscar",
  "version": "1.2.3"
}
"#
    );

    assert!(!is_file(&config_path));

    Ok(())
}

/// Add and remove a usage with `--from-url <file://...>`.
///
/// `--from-url` auto-resolves the URL and writes a `src_path` source into the
/// configuration file, similar to `--path` but driven by the URL resolver.
#[test]
fn add_and_remove_from_url() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir_dep, cwd_dep) = new_project("papa", "add-from-url-dep", "1.2.3")?;

    let (_temp_dir, cwd) = cli_init_json_project(None, "papa", "add-from-url", "1.2.3", None)?;

    let dep_url = file_url_from_path(&cwd_dep);
    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--from-url",
            &dep_url,
            "--iri",
            "urn:kpar:add-from-url-dep",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "  Adding usage: IRI `urn:kpar:add-from-url-dep`
    Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:add-from-url-dep` to configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "add-from-url",
  "publisher": "papa",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:add-from-url-dep"
    }
  ]
}
"#
    );

    let config = fs::read_to_string(&config_path)?;
    assert!(
        config.contains("editable"),
        "config should record an editable source resolved from the file:// URL"
    );
    assert!(config.contains("urn:kpar:add-from-url-dep"));

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:add-from-url-dep"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:add-from-url-dep` from usages
     Removed `urn:kpar:add-from-url-dep`
    Creating env
     Syncing env
             nothing to do: env is already up to date
    Removing source for `urn:kpar:add-from-url-dep` from configuration file at `{config_path}`
    Removing empty configuration file at `{config_path}`"
    )));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "add-from-url",
  "publisher": "papa",
  "version": "1.2.3"
}
"#
    );

    assert!(!is_file(&config_path));

    Ok(())
}

/// `add <URL>` of a kpar served over HTTP installs it into `.sysand`
#[test]
fn add_from_http_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "tango", "add-from-http-kpar", "1.0.0", None)?;

    let test_path = fixture_path("test_lib.kpar");

    let mut server = Server::new();

    let test_body = fs::read(test_path)?;

    let git_mock = server
        .mock("GET", "/test_lib.kpar/info/refs?service=git-upload-pack")
        .with_status(404)
        .expect(1)
        .create();

    let project_mock = server
        .mock("HEAD", "/test_lib.kpar/.project.json")
        .with_status(404)
        .expect(1)
        .create();

    let meta_mock = server
        .mock("HEAD", "/test_lib.kpar/.meta.json")
        .with_status(404)
        .expect(1)
        .create();

    let head_mock = server
        .mock("HEAD", "/test_lib.kpar")
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(&test_body)
        .expect(0)
        .create();

    let get_mock = server
        .mock("GET", "/test_lib.kpar")
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(&test_body)
        .expect(2) // TODO: Reduce this to 1 after caching
        .create();

    let project_url = format!("{}/test_lib.kpar", server.url());

    let out = run_sysand_in(&cwd, ["add", "--iri", &project_url, "--no-index"], None)?;

    head_mock.assert();
    get_mock.assert();
    git_mock.assert();
    project_mock.assert();
    meta_mock.assert();

    out.assert().success();

    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    assert!(env_toml.contains(r#"name = "Lib test""#), "{env_toml}");
    assert!(
        env_toml.contains(r#"path = "lib/127.0.0.1-test_lib_0.0.1""#),
        "{env_toml}"
    );
    assert!(env_toml.contains(&project_url), "{env_toml}");
    assert!(env_toml.contains("kpar_cksum"), "{env_toml}");

    Ok(())
}

/// The full `pkg:sysand/publisher/name` PURL form is stored and removed
/// verbatim.
#[test]
fn add_and_remove_full_purl_sysand_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "quebec", "add-full-purl", "1.2.3", None)?;

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "pkg:sysand/acme-labs/my.project",
        ],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Adding usage: IRI `pkg:sysand/acme-labs/my.project`",
    ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-full-purl",
  "publisher": "quebec",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "pkg:sysand/acme-labs/my.project"
    }
  ]
}
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "pkg:sysand/acme-labs/my.project"],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Removing `pkg:sysand/acme-labs/my.project` from usages
     Removed `pkg:sysand/acme-labs/my.project`",
    ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-full-purl",
  "publisher": "quebec",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

/// A `urn:` IRI whose path segment contains a slash is stored and removed
/// verbatim.
#[test]
fn add_and_remove_urn_with_slash() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "romeo", "urn-slash", "1.2.3", None)?;

    let out = run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--iri", "urn:kpar:acme-labs/my.project"],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Adding usage: IRI `urn:kpar:acme-labs/my.project`",
    ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "urn-slash",
  "publisher": "romeo",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:acme-labs/my.project"
    }
  ]
}
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:acme-labs/my.project"],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Removing `urn:kpar:acme-labs/my.project` from usages
     Removed `urn:kpar:acme-labs/my.project`",
    ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "urn-slash",
  "publisher": "romeo",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

// Disabled: the `env install` subcommand it relies on is commented out in the CLI.
/*
#[test]
fn add_and_remove_with_lock_preinstall() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir_dep, cwd_dep) = cli_init_json_project(
        None,
        "acme",
        "add-and-remove-with-lock-preinstall-dep",
        "1.2.3",
        None,
    )?;

    std::fs::write(
        cwd_dep.join("add-and-remove-with-lock-preinstall-dep.sysml"),
        "package AddAndRemoveWithLockLocalDep;",
    )?;

    run_sysand_in(
        &cwd_dep,
        ["include", "add-and-remove-with-lock-preinstall-dep.sysml"],
        None,
    )?
    .assert()
    .success();

    let (_temp_dir, cwd) = cli_init_json_project(
        None,
        "tango",
        "add-and-remove-with-lock-preinstall",
        "1.2.3",
        None,
    )?;

    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:add-and-remove-with-lock-preinstall-dep",
            "--path",
            cwd_dep.as_str(),
        ],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri", "urn:kpar:add-and-remove-with-lock-preinstall-dep",
            "--no-index",
        ],
        None,
    )?
    .assert()
    .success()
    .stderr(contains(
        "Adding usage: IRI `urn:kpar:add-and-remove-with-lock-preinstall-dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove-with-lock-preinstall",
  "publisher": "tango",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:add-and-remove-with-lock-preinstall-dep"
    }
  ]
}
"#
    );

    run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:add-and-remove-with-lock-preinstall-dep"],
        None,
    )?
    .assert()
    .success();

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add-and-remove-with-lock-preinstall",
  "publisher": "tango",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}
*/

#[test]
fn add_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "add-nonexistent", "1.2.3", None)?;

    // An unreachable index, so the default one is never queried
    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:add-nonexistent",
            "--default-index",
            "http://127.0.0.1:1",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(contains("failed to retrieve project(s)"));

    Ok(())
}

#[test]
fn remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "remove-nonexistent", "1.2.3", None)?;

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:remove-nonexistent"],
        None,
    )?;

    out.assert().failure().stderr(contains(
        "could not find usage for `urn:kpar:remove-nonexistent`",
    ));

    Ok(())
}

/// `add --no-sync` must update the lockfile but must not touch `.sysand` at
/// all (no env created, no install performed).
#[test]
fn add_no_sync_skips_env_sync() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "add-no-sync-app", "1.0.0", None)?;

    let (_tmp_dep, cwd_dep) =
        cli_init_json_project(None, "acme", "add-no-sync-dep", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-sync",
            "--iri",
            "urn:kpar:add-no-sync-dep",
            "--as-local-src",
            cwd_dep.as_str(),
        ],
        cfg,
    )?;

    out.assert()
        .success()
        .stderr(contains("Adding usage: IRI `urn:kpar:add-no-sync-dep`"))
        .stderr(contains("Syncing").not())
        .stderr(contains("Creating env").not());

    let lockfile = fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    assert!(
        lockfile.contains("add-no-sync-dep"),
        "lockfile must still be generated by `add --no-sync`: {lockfile}"
    );

    assert!(
        !exists(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH)),
        "`add --no-sync` must not create the environment"
    );

    Ok(())
}

/// `add` must remove a project from `.sysand` once it is no longer present
/// in the freshly regenerated lockfile, while leaving dependencies that are
/// still needed (and the dependency being added) alone.
#[test]
fn add_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "add-prune-app", "1.0.0", None)?;

    let (_tmp_keep, cwd_keep) =
        cli_init_json_project(None, "acme", "add-prune-keep", "1.0.0", None)?;

    let (_tmp_drop, cwd_drop) =
        cli_init_json_project(None, "acme", "add-prune-drop", "1.0.0", None)?;

    let (_tmp_new, cwd_new) = cli_init_json_project(None, "acme", "add-prune-new", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:add-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:add-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();
    run_sysand_in(&cwd, ["sync"], cfg)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(is_dir(env_lib.join("kpar.add-prune-keep_1.0.0")));
    assert!(is_dir(env_lib.join("kpar.add-prune-drop_1.0.0")));

    // Drop the usage and regenerate the lockfile without touching the env.
    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "--iri", "urn:kpar:add-prune-drop"],
        cfg,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();

    // Adding a new dependency triggers a full relock + sync; by default this
    // must prune `add-prune-drop`, which is no longer in the lockfile.
    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:add-prune-new",
            "--as-local-src",
            cwd_new.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    assert!(
        is_dir(env_lib.join("kpar.add-prune-keep_1.0.0")),
        "still-needed dependency must not be pruned"
    );
    assert!(
        is_dir(env_lib.join("kpar.add-prune-new_1.0.0")),
        "newly added dependency must be installed"
    );
    assert!(
        !exists(env_lib.join("kpar.add-prune-drop_1.0.0")),
        "unneeded dependency must be pruned from `.sysand` by default"
    );

    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    assert!(env_toml.contains("add-prune-keep"));
    assert!(env_toml.contains("add-prune-new"));
    assert!(!env_toml.contains("add-prune-drop"));

    Ok(())
}

/// `add --no-prune` must leave a dependency that is no longer present in the
/// freshly regenerated lockfile installed in `.sysand`.
#[test]
fn add_no_prune_keeps_unneeded_dependency() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "add-no-prune-app", "1.0.0", None)?;

    let (_tmp_keep, cwd_keep) =
        cli_init_json_project(None, "acme", "add-no-prune-keep", "1.0.0", None)?;

    let (_tmp_drop, cwd_drop) =
        cli_init_json_project(None, "acme", "add-no-prune-drop", "1.0.0", None)?;

    let (_tmp_new, cwd_new) =
        cli_init_json_project(None, "acme", "add-no-prune-new", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:add-no-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:add-no-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();
    run_sysand_in(&cwd, ["sync"], cfg)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(is_dir(env_lib.join("kpar.add-no-prune-keep_1.0.0")));
    assert!(is_dir(env_lib.join("kpar.add-no-prune-drop_1.0.0")));

    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "--iri", "urn:kpar:add-no-prune-drop"],
        cfg,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();

    // `--no-prune` must not remove the now-unneeded dependency from `.sysand`.
    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-prune",
            "--iri",
            "urn:kpar:add-no-prune-new",
            "--as-local-src",
            cwd_new.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    assert!(is_dir(env_lib.join("kpar.add-no-prune-keep_1.0.0")));
    assert!(is_dir(env_lib.join("kpar.add-no-prune-new_1.0.0")));
    assert!(
        is_dir(env_lib.join("kpar.add-no-prune-drop_1.0.0")),
        "`--no-prune` must leave the unneeded dependency installed in `.sysand`"
    );

    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    assert!(
        env_toml.contains("add-no-prune-drop"),
        "`--no-prune` must leave the unneeded dependency registered in env.toml"
    );

    Ok(())
}

/// After `remove` updates an existing lockfile, the lockfile must remain
/// internally consistent, and the dependency should be gone from `.sysand`
#[test]
fn remove_keeps_lockfile_valid_and_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "remove-lock-app", "1.0.0", None)?;

    let (_tmp_dep, cwd_dep) =
        cli_init_json_project(None, "acme", "remove-lock-dep", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:remove-lock-dep",
            "--as-local-src",
            cwd_dep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["lock"], cfg)?.assert().success();
    run_sysand_in(&cwd, ["sync"], cfg)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(is_dir(env_lib.join("kpar.remove-lock-dep_1.0.0")));

    run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:remove-lock-dep"], cfg)?
        .assert()
        .success();

    let lockfile = fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    assert!(
        !lockfile.contains("urn:kpar:remove-lock-dep"),
        "lockfile must not reference the removed dependency anywhere, including in the root's own usage list: {lockfile}"
    );

    assert!(
        !exists(env_lib.join("kpar.remove-lock-dep_1.0.0")),
        "the dependency dropped by `remove` must be pruned"
    );

    Ok(())
}

/// `remove` must remove a project from `.sysand` once it is no longer
/// needed, while leaving dependencies that are still needed alone.
#[test]
fn remove_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "remove-prune-app", "1.0.0", None)?;

    let (_tmp_keep, cwd_keep) =
        cli_init_json_project(None, "acme", "remove-prune-keep", "1.0.0", None)?;

    let (_tmp_drop, cwd_drop) =
        cli_init_json_project(None, "acme", "remove-prune-drop", "1.0.0", None)?;

    let (_tmp_extra, cwd_extra) =
        cli_init_json_project(None, "acme", "remove-prune-extra", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:remove-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:remove-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    // Add an unrelated project, then immediately remove its usage without
    // pruning: this leaves it physically installed in `.sysand` while
    // excluded from the current lockfile, mirroring a stale/orphaned
    // install left over from an earlier lockfile.
    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:remove-prune-extra",
            "--as-local-src",
            cwd_extra.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(is_dir(env_lib.join("kpar.remove-prune-extra_1.0.0")));

    run_sysand_in(
        &cwd,
        [
            "remove",
            "--iri",
            "urn:kpar:remove-prune-extra",
            "--no-prune",
        ],
        cfg,
    )?
    .assert()
    .success();

    // Still physically present, but no longer part of the lockfile.
    assert!(is_dir(env_lib.join("kpar.remove-prune-extra_1.0.0")));

    run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:remove-prune-drop"], cfg)?
        .assert()
        .success();

    assert!(
        is_dir(env_lib.join("kpar.remove-prune-keep_1.0.0")),
        "still-needed dependency must be installed"
    );
    assert!(
        !exists(env_lib.join("kpar.remove-prune-extra_1.0.0")),
        "a project not present in the lockfile must be pruned from `.sysand` by default"
    );

    Ok(())
}

/// `remove --no-prune` must leave a project that is not present in the
/// lockfile installed in `.sysand`, while still syncing dependencies that
/// are still needed.
#[test]
fn remove_no_prune_keeps_unneeded_dependency_and_still_syncs()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "remove-no-prune-app", "1.0.0", None)?;

    let (_tmp_keep, cwd_keep) =
        cli_init_json_project(None, "acme", "remove-no-prune-keep", "1.0.0", None)?;

    let (_tmp_drop, cwd_drop) =
        cli_init_json_project(None, "acme", "remove-no-prune-drop", "1.0.0", None)?;

    let (_tmp_extra, cwd_extra) =
        cli_init_json_project(None, "acme", "remove-no-prune-extra", "1.0.0", None)?;

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:remove-no-prune-keep",
            "--as-local-src",
            cwd_keep.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:remove-no-prune-drop",
            "--as-local-src",
            cwd_drop.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    // Add an unrelated project, then immediately remove its usage without
    // pruning: this leaves it physically installed in `.sysand` while
    // excluded from the current lockfile.
    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            "urn:kpar:remove-no-prune-extra",
            "--as-local-src",
            cwd_extra.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    run_sysand_in(
        &cwd,
        [
            "remove",
            "--iri",
            "urn:kpar:remove-no-prune-extra",
            "--no-prune",
        ],
        cfg,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");

    run_sysand_in(
        &cwd,
        [
            "remove",
            "--no-prune",
            "--iri",
            "urn:kpar:remove-no-prune-drop",
        ],
        cfg,
    )?
    .assert()
    .success();

    assert!(
        is_dir(env_lib.join("kpar.remove-no-prune-keep_1.0.0")),
        "`--no-prune` must not skip syncing dependencies that are still needed"
    );
    assert!(
        is_dir(env_lib.join("kpar.remove-no-prune-extra_1.0.0")),
        "`--no-prune` must leave a project not present in the lockfile installed in `.sysand`"
    );

    Ok(())
}

/// Write a KPAR containing `.project.json`/`.meta.json` at the archive root,
/// as required by the `KparPath` usage type.
fn write_dep_kpar(
    kpar_path: &camino::Utf8Path,
    publisher: &str,
    name: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    write_kpar(
        kpar_path,
        &format!(r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}"}}"#),
        r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#,
    )
}

#[test]
fn add_dir_and_remove_identifier_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "acme", "exp-add-and-remove", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        init_project_in(&dep_dir, "Acme Labs", "My Dep", "1.0.0")?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

        out.assert()
            .success()
            .stderr(contains("Adding usage: `Acme Labs/My Dep` from `dep`"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Dep` from `dep`"],
            "{kind}"
        );
        if kind == "project-json" {
            assert_eq!(
                manifest_text(&cwd)?,
                r#"{
  "name": "exp-add-and-remove",
  "publisher": "acme",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "Acme Labs",
      "name": "My Dep"
    }
  ]
}
"#
            );
        }

        let out = run_sysand_in(&cwd, ["remove", "Acme Labs/My Dep"], None)?;

        let assert = out
            .assert()
            .success()
            .stderr(contains("Removing `Acme Labs/My Dep` from usages"));
        // Only a `.project.json` project reports what it removed
        if kind == "project-json" {
            assert.stderr(contains("Removed `Acme Labs/My Dep` (path `dep`)"));
        }

        assert!(declared_usages(&cwd)?.is_empty(), "{kind}");
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn add_dir_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "acme", "exp-add-no-publisher", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        // A hand-written json pair: no `init` writes a project without a publisher
        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        fs::write(
            dep_dir.join(".project.json"),
            r#"{
  "name": "no-publisher-dep",
  "version": "1.0.0"
}
"#,
        )?;
        fs::write(
            dep_dir.join(".meta.json"),
            r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
        )?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

        out.assert()
            .failure()
            .stderr(contains("does not have a publisher"));

        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn add_dir_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("acme", "exp-add-nonexistent", "1.2.3")?;

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    // dep exists as a directory but holds no project

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

    out.assert()
        .failure()
        .stderr(contains("unable to find interchange project"));

    Ok(())
}

#[test]
fn add_dir_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "exp-add-already-present", "1.2.3")?;

        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        init_project_in(&dep_dir, "Acme Labs", "My Dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
            .assert()
            .success();
        let added = manifest_text(&cwd)?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

        out.assert()
            .success()
            .stderr(contains("is already present"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Dep` from `dep`"],
            "{kind}"
        );
        assert_eq!(manifest_text(&cwd)?, added, "{kind}");
        if kind == "project-json" {
            assert_eq!(
                added,
                r#"{
  "name": "exp-add-already-present",
  "publisher": "acme",
  "version": "1.2.3",
  "usage": [
    {
      "dir": "dep",
      "publisher": "Acme Labs",
      "name": "My Dep"
    }
  ]
}
"#
            );
        }
    }

    Ok(())
}

#[test]
fn remove_identifier() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "acme", "exp-remove", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        init_project_in(&dep_dir, "Acme Labs", "My Dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
            .assert()
            .success();

        let out = run_sysand_in(&cwd, ["remove", "Acme Labs/my-dep"], None)?;

        let assert = out
            .assert()
            .success()
            .stderr(contains("Removing `Acme Labs/my-dep` from usages"));
        // Only a `.project.json` project reports what it removed
        if kind == "project-json" {
            assert.stderr(contains("Removed `Acme Labs/My Dep` (path `dep`)"));
        }

        assert!(declared_usages(&cwd)?.is_empty(), "{kind}");
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn remove_identifier_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("acme", "exp-remove-nonexistent", "1.2.3")?;

    let out = run_sysand_in(&cwd, ["remove", "Acme Labs/Nonexistent"], None)?;

    out.assert()
        .failure()
        .stderr(contains("could not find usage for `Acme Labs/Nonexistent`"));

    Ok(())
}

#[test]
fn add_kpar_path_and_remove_identifier_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "exp-add-and-remove-kpar-path", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_kpar = cwd.join("dep.kpar");
        write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

        out.assert()
            .success()
            .stderr(contains("Adding usage: `Acme Labs/My Dep` in `dep.kpar`"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Dep` in `dep.kpar`"],
            "{kind}"
        );
        if kind == "project-json" {
            assert_eq!(
                manifest_text(&cwd)?,
                r#"{
  "name": "exp-add-and-remove-kpar-path",
  "publisher": "acme",
  "version": "1.2.3",
  "usage": [
    {
      "kparPath": "dep.kpar",
      "publisher": "Acme Labs",
      "name": "My Dep"
    }
  ]
}
"#
            );
        }

        let out = run_sysand_in(&cwd, ["remove", "Acme Labs/My Dep"], None)?;

        let assert = out
            .assert()
            .success()
            .stderr(contains("Removing `Acme Labs/My Dep` from usages"));
        // Only a `.project.json` project reports what it removed
        if kind == "project-json" {
            assert.stderr(contains("Removed `Acme Labs/My Dep` (path `dep.kpar`)"));
        }

        assert!(declared_usages(&cwd)?.is_empty(), "{kind}");
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn add_kpar_path_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "exp-add-kpar-path-no-publisher", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_kpar = cwd.join("dep.kpar");
        write_kpar(
            &dep_kpar,
            r#"{"name":"no-publisher-dep","version":"1.0.0"}"#,
            r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
        )?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

        out.assert()
            .failure()
            .stderr(contains("does not have a publisher"));

        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn add_kpar_path_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("acme", "exp-add-kpar-path-nonexistent", "1.2.3")?;

    let dep_kpar = cwd.join("dep.kpar");
    // dep.kpar exists as a valid, but empty, archive: no project in it
    {
        let file = fs::File::create(&dep_kpar)?;
        zip::ZipWriter::new(file).finish()?;
    }

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(contains("unable to find interchange project"));

    Ok(())
}

#[test]
fn add_kpar_path_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "exp-add-kpar-path-already-present", "1.2.3")?;

        let dep_kpar = cwd.join("dep.kpar");
        write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?
            .assert()
            .success();
        let added = manifest_text(&cwd)?;

        let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

        out.assert()
            .success()
            .stderr(contains("is already present"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Dep` in `dep.kpar`"],
            "{kind}"
        );
        assert_eq!(manifest_text(&cwd)?, added, "{kind}");
        if kind == "project-json" {
            assert_eq!(
                added,
                r#"{
  "name": "exp-add-kpar-path-already-present",
  "publisher": "acme",
  "version": "1.2.3",
  "usage": [
    {
      "kparPath": "dep.kpar",
      "publisher": "Acme Labs",
      "name": "My Dep"
    }
  ]
}
"#
            );
        }
    }

    Ok(())
}

#[test]
fn remove_identifier_kpar_path() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "acme", "exp-remove-kpar-path", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_kpar = cwd.join("dep.kpar");
        write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?
            .assert()
            .success();

        let out = run_sysand_in(&cwd, ["remove", "acme-labs/my-dep"], None)?;

        let assert = out
            .assert()
            .success()
            .stderr(contains("Removing `acme-labs/my-dep` from usages"));
        // Only a `.project.json` project reports what it removed
        if kind == "project-json" {
            assert.stderr(contains("Removed `Acme Labs/My Dep` (path `dep.kpar`)"));
        }

        assert!(declared_usages(&cwd)?.is_empty(), "{kind}");
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

/// After `remove <publisher>/<name>` updates an existing lockfile, the lockfile
/// must remain internally consistent, and the dependency should be gone
/// from `.sysand`.
#[test]
fn remove_identifier_keeps_lockfile_valid_and_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "exp-remove-lock-app", "1.0.0", None)?;

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    init_project_in(&dep_dir, "Acme Labs", "Remove Lock Dep", "1.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    run_sysand_in(&cwd, ["lock"], None)?.assert().success();
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();

    // A `dir` dependency is used in place: `env.toml` points at it.
    let env_toml = cwd.join(DEFAULT_ENV_NAME).join("env.toml");
    assert!(fs::read_to_string(&env_toml)?.contains("Remove Lock Dep"));

    run_sysand_in(&cwd, ["remove", "Acme Labs/Remove Lock Dep"], None)?
        .assert()
        .success();

    let lockfile = fs::read_to_string(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    assert!(
        !lockfile.contains("Remove Lock Dep") && !lockfile.contains("remove-lock-dep"),
        "lockfile must not reference the removed dependency anywhere, including in the root's own usage list: {lockfile}"
    );

    assert!(
        !fs::read_to_string(&env_toml)?.contains("Remove Lock Dep"),
        "the dependency dropped by `remove <publisher>/<name>` must be pruned"
    );

    Ok(())
}

#[test]
fn remove_identifier_removes_the_sysand_purl_resource() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "acme", "remove-identifier-purl", "1.2.3", None)?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "pkg:sysand/acme-labs/my.project",
        ],
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(&cwd, ["remove", "--no-lock", "acme-labs/my.project"], None)?;

    out.assert()
        .success()
        .stderr(contains("Removing `acme-labs/my.project` from usages"))
        .stderr(contains("Removed `pkg:sysand/acme-labs/my.project`"));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "remove-identifier-purl",
  "publisher": "acme",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn remove_identifier_matches_a_typed_usage_by_the_normalized_form()
-> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "remove-identifier-normalized", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        init_project_in(&dep_dir, "Acme Labs", "My Dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
            .assert()
            .success();

        let out = run_sysand_in(&cwd, ["remove", "--no-lock", "acme-labs/my-dep"], None)?;

        let assert = out.assert().success();
        // Only a `.project.json` project reports what it removed
        if kind == "project-json" {
            assert.stderr(contains("Removed `Acme Labs/My Dep` (path `dep`)"));
        }

        assert!(declared_usages(&cwd)?.is_empty(), "{kind}");
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");
    }

    Ok(())
}

#[test]
fn remove_iri_of_a_typed_usage_suggests_identifier() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "acme", "remove-iri-of-typed", "1.2.3")?;

        let dep_dir = cwd.join("dep");
        fs::create_dir_all(&dep_dir)?;
        // A valid `pkg:sysand` publisher, so the typed usage has a PURL identifier
        init_project_in(&dep_dir, "acme-labs", "my-dep", "1.0.0")?;

        run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
            .assert()
            .success();

        let out = run_sysand_in(
            &cwd,
            ["remove", "--iri", "pkg:sysand/acme-labs/my-dep"],
            None,
        )?;

        let assert = out.assert().failure();
        if kind == "project-json" {
            assert
                .stderr(contains("declared as a directory usage"))
                .stderr(contains(format!(
                    "sysand remove {}",
                    sysand::quote_for_shell("acme-labs/my-dep")
                )));
        } else {
            // A `model.toml` project cannot declare a resource usage at all
            assert.stderr(contains(
                "cannot declare: name the dependency as `<publisher>/<name>`",
            ));
        }
    }

    Ok(())
}

/// Removing a typed usage by its normalized spelling must also remove it
/// from the lockfile. When the spelling is not one an index usage can spell
/// (here the publisher's `.`), the lock records a `urn:sysand` identifier of
/// the normalized spelling, in which the `.` at the end is dropped
#[test]
fn remove_typed_by_normalized_spelling_updates_lockfile() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "remove-norm-app", "1.0.0", None)?;

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    // Written by hand: the spelling is one a json project may carry but
    // no `model.toml` can, which is what the test is about
    write_hand_written_json_project(&dep_dir, "ACME Inc.", "Foo", "1.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-index", "--dir", "dep"], None)?
        .assert()
        .success();

    let lockfile_path = cwd.join(DEFAULT_LOCKFILE_NAME);
    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert!(
        lockfile.contains("urn:sysand:acme-inc/foo"),
        "lockfile must reference the added dependency: {lockfile}"
    );

    run_sysand_in(&cwd, ["remove", "--no-index", "acme-inc/foo"], None)?
        .assert()
        .success()
        .stderr(contains("Removed `ACME Inc./Foo` (path `dep`)"));

    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert!(
        !lockfile.contains("urn:sysand:acme-inc/foo"),
        "lockfile must not reference the removed dependency: {lockfile}"
    );

    Ok(())
}

/// Identifiers that are not a valid `<publisher>/<name>` pair, with the
/// expected error message
const INVALID_IDENTIFIERS: &[(&str, &str)] = &[
    (
        "acme-labs",
        "identifier is not of the form `<publisher>/<name>`",
    ),
    ("/my.project", "publisher cannot be empty"),
    ("acme-labs/", "name cannot be empty"),
    ("acme-labs/my/project", "name cannot contain `/`"),
    ("acme:labs/my.project", "publisher cannot contain `:`"),
    ("acme-labs/my:project", "name cannot contain `:`"),
    ("acme\tlabs/my.project", "publisher cannot contain `\\t`"),
    ("acme-labs/my\nproject", "name cannot contain `\\n`"),
];

/// Identifiers `add` refuses: it adds an index usage, so publisher and name
/// must be spelled as an index usage can spell them, a stricter rule than
/// the one `remove` checks (see [`INVALID_IDENTIFIERS`])
const INVALID_INDEX_IDENTIFIERS: &[(&str, &str)] = &[
    (
        "acme-labs",
        "identifier is not of the form `<publisher>/<name>`",
    ),
    (
        "/my.project",
        "publisher must be 3-50 ASCII alphanumeric chars",
    ),
    ("acme-labs/", "name must be 3-50 ASCII alphanumeric chars"),
    (
        "acme-labs/my/project",
        "name must be 3-50 ASCII alphanumeric chars",
    ),
    (
        "acme:labs/my.project",
        "publisher must be 3-50 ASCII alphanumeric chars",
    ),
    (
        "acme-labs/my:project",
        "name must be 3-50 ASCII alphanumeric chars",
    ),
    (
        "acme\tlabs/my.project",
        "publisher must be 3-50 ASCII alphanumeric chars",
    ),
    (
        "acme-labs/my\nproject",
        "name must be 3-50 ASCII alphanumeric chars",
    ),
    // Valid for a directory or KPAR usage, but not for an index usage
    (
        "Foo & Bar/my.project",
        "publisher must be 3-50 ASCII alphanumeric chars",
    ),
    (
        "acme-labs/my_project",
        "name must be 3-50 ASCII alphanumeric chars",
    ),
];

/// A typed `sysand add <identifier>` should reject an invalid identifier
/// while parsing arguments, before adding anything
#[test]
fn add_identifier_rejects_invalid_identifiers() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "add-identifier-invalid", "1.2.3")?;
        let original = manifest_text(&cwd)?;

        for &(identifier, msg) in INVALID_INDEX_IDENTIFIERS {
            let out = run_sysand_in(&cwd, ["add", "--no-lock", identifier], None)?;

            out.assert().failure().stderr(
                contains(format!("invalid value '{identifier}' for '[IDENTIFIER]'"))
                    .and(contains(msg)),
            );
            assert_eq!(
                manifest_text(&cwd)?,
                original,
                "kind: {kind}, identifier: {identifier:?}"
            );
        }
    }

    Ok(())
}

/// `sysand add <identifier>` should accept spaces in publisher and name,
/// reaching the typed add
#[test]
fn add_identifier_accepts_spaces() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("acme", "add-identifier-spaces", "1.2.3")?;

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "Acme Labs/My Project"], None)?;

    out.assert()
        .failure()
        .stderr(contains("an index usage needs a version constraint"));

    Ok(())
}

/// `sysand remove <identifier>` should reject an invalid identifier while
/// parsing arguments, leaving the project untouched
#[test]
fn remove_identifier_rejects_invalid_identifiers() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "acme", "remove-identifier-invalid", "1.2.3")?;
        let original = manifest_text(&cwd)?;

        for &(identifier, msg) in INVALID_IDENTIFIERS {
            let out = run_sysand_in(&cwd, ["remove", "--no-lock", identifier], None)?;

            out.assert().failure().stderr(
                contains(format!("invalid value '{identifier}' for '[IDENTIFIER]'"))
                    .and(contains(msg)),
            );
            assert_eq!(
                manifest_text(&cwd)?,
                original,
                "kind: {kind}, identifier: {identifier:?}"
            );
        }
    }

    Ok(())
}

/// Removing a usage whose project is still needed transitively must drop it
/// from the root's usages in the lockfile, even though no project is pruned
#[test]
fn remove_still_needed_dependency_updates_root_usages_in_lockfile()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "remove-needed-app", "1.0.0", None)?;

    for name in ["dep-a", "dep-b"] {
        let dir = cwd.join(name);
        fs::create_dir_all(&dir)?;
        init_project_in(&dir, "acme", name, "1.0.0")?;
    }
    run_sysand_in(
        &cwd.join("dep-a"),
        ["add", "--no-lock", "--dir", "../dep-b"],
        None,
    )?
    .assert()
    .success();
    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep-a"], None)?
        .assert()
        .success();
    run_sysand_in(&cwd, ["add", "--no-index", "--dir", "dep-b"], None)?
        .assert()
        .success();

    let root_usages = |lock: &Lock| -> Vec<String> {
        let root = lock
            .projects
            .iter()
            .find(|p| p.name == "remove-needed-app")
            .unwrap();
        root.usages.iter().map(|u| u.to_string()).collect()
    };
    let lock = Lockfile::read(&cwd)?.into_lock();
    assert_eq!(
        root_usages(&lock),
        ["pkg:sysand/acme/dep-a", "pkg:sysand/acme/dep-b"]
    );

    run_sysand_in(&cwd, ["remove", "--no-index", "acme/dep-b"], None)?
        .assert()
        .success();

    let lock = Lockfile::read(&cwd)?.into_lock();
    assert_eq!(root_usages(&lock), ["pkg:sysand/acme/dep-a"]);
    assert!(
        lock.projects.iter().any(|p| p.name == "dep-b"),
        "`dep-b` is still needed by `dep-a`, so it must stay locked: {lock:?}"
    );

    Ok(())
}

#[test]
fn add_publisher_name_writes_an_index_usage() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "foxtrot", "add-index-usage", "1.2.3")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

        let out = run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "Acme Labs/My Lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?;
        out.assert()
            .success()
            .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1)"));

        // Adding the same spelling again changes nothing
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "Acme Labs/My Lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success();

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Lib` (^1)"],
            "{kind}"
        );
        let manifest = manifest_text(&cwd)?;
        if kind == "project-json" {
            assert_eq!(
                manifest,
                r#"{
  "name": "add-index-usage",
  "publisher": "foxtrot",
  "version": "1.2.3",
  "usage": [
    {
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^1"
    }
  ]
}
"#
            );
        }

        // A different spelling of the same project is refused
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "Acme labs/My Lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .failure()
        .stderr(contains(
            "`Acme labs/My Lib` is already declared as an index usage `Acme Labs/My Lib`;\n\
             a typed usage must spell the publisher and name exactly as the project does",
        ));
        assert_eq!(manifest_text(&cwd)?, manifest, "{kind}");
    }

    Ok(())
}

#[test]
fn add_index_usage_without_lock_needs_a_constraint() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("foxtrot", "add-index-no-constraint", "1.2.3")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "acme/lib"], None)?
        .assert()
        .failure()
        .stderr(contains("an index usage needs a version constraint"));

    Ok(())
}

#[test]
fn add_rejects_an_invalid_publisher() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("golf", "reject-add-shorthand", "1.2.3")?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "A/My.Project",
            "--version-constraint",
            "^1",
        ],
        None,
    )?
    .assert()
    .failure()
    .stderr(contains(
        "invalid value 'A/My.Project' for '[IDENTIFIER]': publisher must be 3-50",
    ));

    Ok(())
}

/// Migrating a legacy PURL usage means removing it first
#[test]
fn add_index_usage_over_a_legacy_purl_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "golf", "add-index-over-purl", "1.2.3", None)?;

    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--iri", "pkg:sysand/acme-labs/my-lib"],
        None,
    )?
    .assert()
    .success();
    for args in [
        &[
            "add",
            "--no-lock",
            "Acme Labs/My Lib",
            "--version-constraint",
            "^1",
        ][..],
        &["add", "Acme Labs/My Lib"][..],
    ] {
        run_sysand_in(&cwd, args.iter().copied(), None)?
            .assert()
            .failure()
            .stderr(contains(
                "`pkg:sysand/acme-labs/my-lib` is already declared as a resource usage;\n\
                 remove it before adding it as an index usage",
            ));
    }

    Ok(())
}

#[test]
fn remove_index_usage_by_exact_spelling() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "hotel", "remove-index-usage", "1.2.3", None)?;
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "Acme Labs/My Lib",
            "--version-constraint",
            "^1",
        ],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["remove", "--no-lock", "acme labs/my lib"], None)?
        .assert()
        .failure()
        .stderr(contains(
            "could not find usage for `acme labs/my lib`; did you mean `Acme Labs/My Lib`?",
        ));
    run_sysand_in(
        &cwd,
        [
            "remove",
            "--no-lock",
            "--iri",
            "pkg:sysand/acme-labs/my-lib",
        ],
        None,
    )?
    .assert()
    .failure()
    .stderr(contains(format!(
        "remove it with `sysand remove {}`",
        sysand::quote_for_shell("Acme Labs/My Lib")
    )));

    run_sysand_in(&cwd, ["remove", "--no-lock", "Acme Labs/My Lib"], None)?
        .assert()
        .success()
        .stderr(contains(
            "Removed `Acme Labs/My Lib` with version constraints `^1`",
        ));

    let info_json = fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "remove-index-usage",
  "publisher": "hotel",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_path_like_identifier_suggests_path_options() -> Result<(), Box<dyn std::error::Error>> {
    // Not an identifier at all, or with a publisher or name no project can have
    for path in ["a/b/c", "lib", "/abs/lib", "./lib", "../lib"] {
        let (_temp_dir, _cwd, out) = run_sysand(["add", "--no-lock", path], None)?;

        out.assert()
            .failure()
            .stderr(contains(format!("invalid value '{path}' for '[IDENTIFIER]'")))
            .stderr(contains(
                "to add from a directory, a KPAR or an IRI, use `--dir`, `--kpar-path` or `--iri` respectively",
            ));
    }

    Ok(())
}

#[test]
fn remove_path_like_identifier_suggests_iri_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["remove", "../lib/dep"], None)?;

    out.assert().failure().stderr(contains(
        "to remove a usage by IRI, use `--iri` or `--iri-path`",
    ));

    Ok(())
}

/// When removing a project that is not used, the error message must name
/// it as given
#[test]
fn remove_nonexistent_identifier() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("acme", "remove-nonexistent-identifier", "1.2.3")?;

    let out = run_sysand_in(&cwd, ["remove", "acme-labs/nonexistent"], None)?;

    out.assert()
        .failure()
        .stderr(contains("could not find usage for `acme-labs/nonexistent`"));

    Ok(())
}

/// With no constraint, `add` writes `^` the version that locking chose, here
/// from the local environment. A prerelease is not chosen without a
/// constraint naming one
#[test]
fn add_index_usage_constrains_to_the_locked_version() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "main", "add-index-highest", "1.2.3")?;
        for version in ["1.0.0", "1.2.0", "2.0.0-dev"] {
            install_in_env(&cwd, "Acme Labs", "My Lib", version)?;
        }

        run_sysand_in(
            &cwd,
            ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
            None,
        )?
        .assert()
        .success()
        .stderr(contains("Settled usage: `Acme Labs/My Lib` (^1.2.0)"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Lib` (^1.2.0)"],
            "{kind}"
        );
        assert_json_index_usage(kind, &cwd, "Acme Labs", "My Lib", "^1.2.0")?;
        let lock = fs::read_to_string(cwd.join("sysand-lock.toml"))?;
        assert!(lock.contains(r#"version = "1.2.0""#), "{lock}");
    }

    Ok(())
}

/// The version that locking chooses takes the rest of the dependency graph
/// into account: here a dependency requires `^1`, so `^1.4.0` is written
/// although 2.0.0 is available
#[test]
fn add_index_usage_constrains_to_a_version_the_graph_allows()
-> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "main", "add-index-graph", "1.2.3")?;
        for version in ["1.4.0", "2.0.0"] {
            install_in_env(&cwd, "Acme Labs", "My Lib", version)?;
        }
        install_in_env_with_usage(
            &cwd,
            "Acme Labs",
            "Mid",
            "1.0.0",
            &[r#"{"publisher": "Acme Labs", "name": "My Lib", "versionConstraint": "^1"}"#],
        )?;
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-sync",
                "--no-index",
                "Acme Labs/Mid",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success();

        run_sysand_in(
            &cwd,
            ["add", "--no-sync", "--no-index", "acme-labs/my-lib"],
            None,
        )?
        .assert()
        .success()
        .stderr(contains("Settled usage: `Acme Labs/My Lib` (^1.4.0)"));

        assert!(
            declared_usages(&cwd)?.contains(&"`Acme Labs/My Lib` (^1.4.0)".to_owned()),
            "{kind}"
        );
        assert_json_index_usage(kind, &cwd, "Acme Labs", "My Lib", "^1.4.0")?;
    }

    Ok(())
}

/// A spelling that differs from the project's own is caught by locking, and
/// the manifest is restored
#[test]
fn add_index_usage_spelled_unlike_the_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "main", "add-index-misspelled", "1.2.3")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
        let before = manifest_text(&cwd)?;

        for args in [
            &["add", "--no-sync", "--no-index", "Acme labs/My Lib"][..],
            &[
                "add",
                "--no-sync",
                "--no-index",
                "Acme labs/My Lib",
                "--version-constraint",
                "^1",
            ][..],
        ] {
            run_sysand_in(&cwd, args.iter().copied(), None)?
                .assert()
                .failure()
                .stderr(contains(
                    "index usage `Acme labs/My Lib` in `add-index-misspelled` 1.2.3 resolved to \
                     version 1.0.0 of `Acme Labs/My Lib`, but is rejected because its spelling does \
                     not match the project's;\nspell the usage exactly as `Acme Labs/My Lib`",
                ));
            assert_eq!(manifest_text(&cwd)?, before, "{kind}");
        }
    }

    Ok(())
}

/// A normalized spelling takes the spelling of the project resolved, with or
/// without a version constraint
#[test]
fn add_normalized_index_usage_takes_the_resolved_spelling() -> Result<(), Box<dyn std::error::Error>>
{
    for kind in KINDS {
        for (constraint, written) in [(Some("^1"), "^1"), (None, "^1.0.0")] {
            let (_temp_dir, cwd) =
                new_project_of_kind(kind, "main", "add-index-normalized", "1.2.3")?;
            install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

            let mut args = vec!["add", "--no-sync", "--no-index", "acme-labs/my-lib"];
            if let Some(constraint) = constraint {
                args.extend(["--version-constraint", constraint]);
            }
            run_sysand_in(&cwd, args, None)?
                .assert()
                .success()
                .stderr(contains(format!(
                    "Settled usage: `Acme Labs/My Lib` ({written})"
                )));

            assert_eq!(
                declared_usages(&cwd)?,
                [format!("`Acme Labs/My Lib` ({written})")],
                "{kind}"
            );
            assert_json_index_usage(kind, &cwd, "Acme Labs", "My Lib", written)?;
            assert!(!manifest_text(&cwd)?.contains("acme-labs"), "{kind}");
        }
    }

    Ok(())
}

/// Source overrides are only for resource usages
#[test]
fn add_index_usage_refuses_a_source_override() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("main", "add-index-override", "1.2.3")?;

    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "Acme Labs/My Lib", "--from-path", "dep"],
        None,
    )?
    .assert()
    .failure()
    .stderr(contains("cannot be used with"));

    Ok(())
}

/// A source override of the identifier an index usage shares does not apply
/// to the index usage: overrides are keyed by the IRIs of resource usages
#[test]
fn source_override_does_not_apply_to_an_index_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("main", "override-not-index", "1.2.3")?;
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    fs::create_dir_all(cwd.join("dep"))?;
    write_json_project(&cwd.join("dep"), "Acme Labs", "My Lib", "9.0.0", None)?;
    let config_path = cwd.join("sysand.toml");
    fs::write(
        &config_path,
        r#"[[project]]
identifiers = ["pkg:sysand/acme-labs/my-lib"]
sources = [{ src_path = "dep" }]
"#,
    )?;

    run_sysand_in(
        &cwd,
        ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success()
    .stderr(contains("Settled usage: `Acme Labs/My Lib` (^1.0.0)"));

    let lock = fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "1.0.0""#), "{lock}");
    assert!(!lock.contains(r#"version = "9.0.0""#), "{lock}");

    Ok(())
}

/// Known issue: the solver coalesces usages by identifier, and the first one
/// resolved supplies the candidates for all of them. Here the root's overridden resource usage is
/// resolved before its dependency's index usage of the same project, so the
/// index usage is answered by the override too. Update this test when the
/// coalescing policy is decided
#[test]
fn source_override_reaches_an_index_usage_of_the_same_project_through_coalescing()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) =
        cli_init_json_project(None, "main", "override-coalesced", "1.2.3", None)?;
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    fs::create_dir_all(cwd.join("dep"))?;
    write_json_project(&cwd.join("dep"), "Acme Labs", "My Lib", "9.0.0", None)?;
    fs::create_dir_all(cwd.join("mid"))?;
    let mid = cwd.join("mid");
    init_project_in(&mid, "Acme Labs", "Mid", "1.0.0")?;
    // `^9` is satisfied only by the override
    install_in_env(&mid, "Acme Labs", "My Lib", "9.0.0")?;
    run_sysand_in(
        &mid,
        [
            "add",
            "--no-lock",
            "Acme Labs/My Lib",
            "--version-constraint",
            "^9",
        ],
        None,
    )?
    .assert()
    .success();

    let config_path = cwd.join("sysand.toml");
    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "pkg:sysand/acme-labs/my-lib",
            "--from-path",
            "dep",
        ],
        Some(config_path.as_str()),
    )?
    .assert()
    .success();
    run_sysand_in(
        &cwd,
        ["add", "--no-sync", "--no-index", "--dir", "mid"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success();

    // `mid`'s index usage `^9` is satisfied only by the override's 9.0.0
    let lock = fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "9.0.0""#), "{lock}");

    Ok(())
}

/// Without locking, a normalized spelling takes the spelling of the version
/// installed in the local environment
#[test]
fn add_normalized_index_usage_without_lock_takes_the_installed_spelling()
-> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "foxtrot", "add-index-env-spelling", "1.2.3")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "acme-labs/my-lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success()
        .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1)"));

        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Lib` (^1)"],
            "{kind}"
        );
        assert_json_index_usage(kind, &cwd, "Acme Labs", "My Lib", "^1")?;
        let manifest = manifest_text(&cwd)?;

        // Adding it again the normalized way takes the declared spelling
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "acme-labs/my-lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success();
        assert_eq!(manifest_text(&cwd)?, manifest, "{kind}");
    }

    Ok(())
}

/// Without locking, a spelling that cannot be checked against, or recovered
/// from, the local environment is refused
#[test]
fn add_index_usage_without_lock_needs_it_installed() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "foxtrot", "add-index-not-installed", "1.2.3")?;
        let before = manifest_text(&cwd)?;

        for (spelling, message) in [
            (
                "Acme Labs/My Lib",
                "cannot check that `Acme Labs/My Lib` is spelled as the project spells it",
            ),
            (
                "acme-labs/my-lib",
                "cannot find how the project `acme-labs/my-lib` spells its publisher and name",
            ),
        ] {
            run_sysand_in(
                &cwd,
                ["add", "--no-lock", spelling, "--version-constraint", "^1"],
                None,
            )?
            .assert()
            .failure()
            .stderr(contains(format!(
                "{message}: it is not installed in the local environment"
            )))
            .stderr(contains("leave out `--no-lock`"));
        }
        assert_eq!(manifest_text(&cwd)?, before, "{kind}");

        // Any installed version gives the spelling, whatever the constraint
        install_in_env(&cwd, "Acme Labs", "My Lib", "2.0.0")?;
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "Acme Labs/My Lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success();
    }

    Ok(())
}

/// Without locking, the spelling is that of a version installed in the
/// local environment, whatever the constraint: a project spells itself the
/// same way in every version
#[test]
fn add_index_usage_without_lock_takes_the_installed_spelling_whatever_the_constraint()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project("foxtrot", "add-index-any-version", "1.2.3")?;
    // Not accepted by the constraint
    install_in_env(&cwd, "ACME Labs", "My Lib", "2.0.0")?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "acme-labs/my-lib",
            "--version-constraint",
            "^1",
        ],
        None,
    )?
    .assert()
    .success()
    .stderr(contains("Adding usage: `ACME Labs/My Lib` (^1)"));

    // An exact spelling is checked against it, in a project that does not
    // declare the usage yet
    let (_other_dir, cwd) = new_project("foxtrot", "add-index-any-version-exact", "1.2.3")?;
    install_in_env(&cwd, "ACME Labs", "My Lib", "2.0.0")?;
    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "Acme Labs/My Lib",
            "--version-constraint",
            "^1",
        ],
        None,
    )?
    .assert()
    .failure()
    .stderr(contains(
        "version 2.0.0 installed in the local environment declares itself `ACME Labs/My Lib`",
    ));

    Ok(())
}

/// Adding a usage that is already present still locks and syncs, since the
/// environment may be missing or stale
#[test]
fn add_already_present_usage_still_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "main", "add-present-syncs", "1.2.3", None)?;
    fs::create_dir_all(cwd.join("dep"))?;
    init_project_in(&cwd.join("dep"), "Acme", "Dep", "1.0.0")?;
    let config_path = cwd.join("sysand.toml");
    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-index",
            "--iri",
            "urn:kpar:dep",
            "--from-path",
            "dep",
        ],
        Some(config_path.as_str()),
    )?
    .assert()
    .success();
    let env = cwd.join(DEFAULT_ENV_NAME);
    assert!(is_dir(&env));
    fs::remove_dir_all(&env)?;

    run_sysand_in(
        &cwd,
        ["add", "--no-index", "--iri", "urn:kpar:dep"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success()
    .stderr(contains("since it is already present"));

    assert!(is_dir(&env), "the environment must be synced again");

    Ok(())
}

/// Adding an index usage that is already present, without a constraint,
/// leaves it as it is but still locks
#[test]
fn add_already_present_index_usage_still_locks() -> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) = new_project_of_kind(kind, "main", "add-present-locks", "1.2.3")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
        run_sysand_in(
            &cwd,
            ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
            None,
        )?
        .assert()
        .success();
        let manifest = manifest_text(&cwd)?;
        let lock_path = cwd.join("sysand-lock.toml");
        fs::remove_file(&lock_path)?;

        run_sysand_in(
            &cwd,
            ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
            None,
        )?
        .assert()
        .success()
        .stderr(contains("since it is already present"));

        assert_eq!(manifest_text(&cwd)?, manifest, "{kind}");
        assert!(is_file(&lock_path), "the lockfile must be written again");
    }

    Ok(())
}

/// Without locking, an index usage already declared gives the spelling, so
/// nothing has to be installed to re-add it, by its normalized spelling too
#[test]
fn add_without_lock_of_a_declared_index_usage_needs_nothing_installed()
-> Result<(), Box<dyn std::error::Error>> {
    for kind in KINDS {
        let (_temp_dir, cwd) =
            new_project_of_kind(kind, "foxtrot", "add-declared-no-lock", "1.2.3")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "Acme Labs/My Lib",
                "--version-constraint",
                "^1",
            ],
            None,
        )?
        .assert()
        .success();
        fs::remove_dir_all(cwd.join(DEFAULT_ENV_NAME))?;

        run_sysand_in(
            &cwd,
            [
                "add",
                "--no-lock",
                "acme-labs/my-lib",
                "--version-constraint",
                "<1.5",
            ],
            None,
        )?
        .assert()
        .success();
        assert_eq!(
            declared_usages(&cwd)?,
            ["`Acme Labs/My Lib` (<1.5)"],
            "{kind}"
        );
        assert_json_index_usage(kind, &cwd, "Acme Labs", "My Lib", "<1.5")?;
        let manifest = manifest_text(&cwd)?;

        // Without a constraint, the declared one is kept
        run_sysand_in(&cwd, ["add", "--no-lock", "acme-labs/my-lib"], None)?
            .assert()
            .success()
            .stderr(contains("since it is already present"));
        assert_eq!(manifest_text(&cwd)?, manifest, "{kind}");
    }

    Ok(())
}

// --- `model.toml` projects --------------------------------------------

/// `sysand init --kind model-toml --publisher acme --name <name> --version
/// 1.0.0` in `dir`
fn init_toml_project(dir: &camino::Utf8Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    run_sysand_in(
        dir,
        [
            "init",
            "--kind",
            "model-toml",
            "--publisher",
            "acme",
            "--name",
            name,
            "--version",
            "1.0.0",
            "--metamodel",
            "sysml",
        ],
        None,
    )?
    .assert()
    .success();
    Ok(())
}

/// `add --dir` produces a typed usage, one of the two shapes a manifest
/// can hold without resolving anything.
#[test]
fn add_dir_on_a_toml_project() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    init_toml_project(&cwd, "app")?;
    let dep = cwd.join("dep");
    fs::create_dir_all(&dep)?;
    init_toml_project(&dep, "lib")?;

    // With the default lock follow-up, so the rollback backup is taken
    // of the `model.toml`.
    run_sysand_in(&cwd, ["add", "--no-index", "--dir", "dep"], None)?
        .assert()
        .success();

    let manifest = fs::read_to_string(cwd.join("model.toml"))?;
    assert!(
        manifest.contains("[[inline-usages]]"),
        "manifest: {manifest}"
    );
    assert!(manifest.contains("dir = \"dep\""), "manifest: {manifest}");
    assert!(!exists(cwd.join(".project.json")));

    // And it comes back out by identity.
    run_sysand_in(&cwd, ["remove", "--no-index", "acme/lib"], None)?
        .assert()
        .success();
    assert!(!fs::read_to_string(cwd.join("model.toml"))?.contains("dir ="));

    Ok(())
}

/// A shape a manifest cannot express is refused, and nothing is left
/// behind: neither a half-edited manifest nor a `sysand.toml` override for a
/// usage that was never added.
#[test]
fn a_refused_add_leaves_nothing_behind() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    init_toml_project(&cwd, "app")?;
    let before = fs::read_to_string(cwd.join("model.toml"))?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--iri",
            "urn:kpar:something",
            "--as-local-src",
            "dep",
        ],
        None,
    )?
    .assert()
    .failure();

    assert_eq!(before, fs::read_to_string(cwd.join("model.toml"))?);
    assert!(
        !exists(cwd.join("sysand.toml")),
        "an override was left behind"
    );
    Ok(())
}

/// An index usage needs a constraint the shorthand does not carry.
#[test]
fn adding_an_index_usage_without_a_constraint_says_so() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    init_toml_project(&cwd, "app")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "acme/lib"], None)?
        .assert()
        .failure()
        .stderr(contains("needs a version constraint"));
    Ok(())
}

/// With one, it works. Without locking, the spelling is checked against
/// what is installed, so the project is installed first.
#[test]
fn adding_an_index_usage_with_a_constraint_writes_it() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    init_toml_project(&cwd, "app")?;
    install_in_env(&cwd, "acme", "lib", "1.2.0")?;

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "acme/lib",
            "--version-constraint",
            "^1.2",
        ],
        None,
    )?
    .assert()
    .success();

    let manifest = fs::read_to_string(cwd.join("model.toml"))?;
    assert!(manifest.contains("name = \"lib\""), "manifest: {manifest}");
    assert!(
        manifest.contains("version = \"^1.2\""),
        "manifest: {manifest}"
    );
    Ok(())
}

/// On a `model.toml` project, the shorthand is written to the manifest as the
/// locked project spells itself.
#[test]
fn add_normalized_index_usage_takes_the_resolved_spelling_on_a_toml_project()
-> Result<(), Box<dyn std::error::Error>> {
    for (constraint, written) in [(Some("^1"), "^1"), (None, "^1.0.0")] {
        let (_temp_dir, cwd) = new_temp_cwd()?;
        init_toml_project(&cwd, "app")?;
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

        let mut args = vec!["add", "--no-sync", "--no-index", "acme-labs/my-lib"];
        if let Some(constraint) = constraint {
            args.extend(["--version-constraint", constraint]);
        }
        run_sysand_in(&cwd, args, None)?
            .assert()
            .success()
            .stderr(contains(format!(
                "Settled usage: `Acme Labs/My Lib` ({written})"
            )));

        let manifest = fs::read_to_string(cwd.join("model.toml"))?;
        assert!(manifest.contains("publisher = \"Acme Labs\""), "{manifest}");
        assert!(manifest.contains("name = \"My Lib\""), "{manifest}");
        assert!(
            manifest.contains(&format!("version = \"{written}\"")),
            "{manifest}"
        );
        assert!(!manifest.contains("acme-labs"), "{manifest}");
    }

    Ok(())
}

/// A standard-library usage declares no `publisher`, and is reached by the
/// identity `(omg, <name>)`, so `sysand remove omg/<name>` removes it.
#[test]
fn a_stdlib_usage_is_removed_by_its_omg_identity() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    init_toml_project(&cwd, "app")?;

    let manifest_path = cwd.join("model.toml");
    let manifest = fs::read_to_string(&manifest_path)?;
    fs::write(
        &manifest_path,
        format!(
            r#"{manifest}
[[inline-usages]]
name = "Kernel Semantic Library"
kpar-url = "https://www.omg.org/spec/KerML/20250201/Semantic-Library.kpar"
"#
        ),
    )?;

    run_sysand_in(
        &cwd,
        ["remove", "--no-index", "omg/kernel-semantic-library"],
        None,
    )?
    .assert()
    .success();
    let after = fs::read_to_string(&manifest_path)?;
    assert!(!after.contains("kpar-url"), "manifest: {after}");

    Ok(())
}

/// A hand-commented `model.toml` survives `add` and `remove` with
/// comments and key order intact, end to end.
#[test]
fn add_and_remove_keep_the_comments_and_key_order() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_project_of_kind("model-toml", "acme", "widgets", "1.0.0")?;
    // Without locking, an index usage's spelling is checked against what
    // is installed, so the project is installed first.
    install_in_env(&cwd, "acme", "other", "1.0.0")?;
    let path = cwd.join("model.toml");
    let commented = fs::read_to_string(&path)?
        .replace("[project]", "# who ships it\n[project]")
        .replace("sources = []", "sources = []  # the files\n");
    fs::write(&path, &commented)?;

    let key_names = |text: &str| -> Vec<String> {
        text.lines()
            .filter_map(|line| {
                let line = line.trim();
                (!line.starts_with('#') && line.contains(" = "))
                    .then(|| line.split(" = ").next().unwrap_or_default().to_owned())
            })
            .collect()
    };
    let before = key_names(&commented);

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "acme/other",
            "--version-constraint",
            "^1.0.0",
        ],
        None,
    )?
    .assert()
    .success();
    let added = fs::read_to_string(&path)?;
    assert!(
        added.contains("# who ships it"),
        "add keeps comments:\n{added}"
    );
    assert!(
        added.contains("# the files"),
        "add keeps comments:\n{added}"
    );

    run_sysand_in(&cwd, ["remove", "acme/other"], None)?
        .assert()
        .success();
    let removed = fs::read_to_string(&path)?;
    assert!(
        removed.contains("# who ships it") && removed.contains("# the files"),
        "remove keeps comments:\n{removed}"
    );
    assert_eq!(
        before,
        key_names(&removed),
        "add then remove must leave the key order as it was"
    );
    Ok(())
}
