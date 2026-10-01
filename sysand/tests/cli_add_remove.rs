// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::fs;

use assert_cmd::prelude::*;
use mockito::Server;
use predicates::prelude::{predicate::str::contains, *};
use sysand_core::env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

#[test]
fn add_and_remove_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("e", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "urn:kpar:test"], None)?;

    out.assert()
        .success()
        .stderr(contains("Adding usage: IRI `urn:kpar:test`"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "e",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:test"], None)?;

    out.assert().success().stderr(contains(
        "Removing `urn:kpar:test` from usages
     Removed `urn:kpar:test`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "e",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_publisher_name_writes_an_index_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_usage", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "Acme Labs/My Lib", "^1"], None)?;
    out.assert()
        .success()
        .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1)"));

    // Adding the same spelling again changes nothing
    run_sysand_in(&cwd, ["add", "--no-lock", "Acme Labs/My Lib", "^1"], None)?
        .assert()
        .success();

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "add_index_usage",
  "publisher": "f",
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

    // A different spelling of the same project is refused
    run_sysand_in(&cwd, ["add", "--no-lock", "Acme labs/My Lib", "^1"], None)?
        .assert()
        .failure()
        .stderr(contains(
            "index usage `Acme labs/My Lib` is rejected because its spelling does not match \
             the project's: version 1.0.0 installed in the local environment declares itself \
             `Acme Labs/My Lib`;\nspell the usage exactly as `Acme Labs/My Lib`",
        ));
    assert_eq!(
        std::fs::read_to_string(cwd.join(".project.json"))?,
        info_json
    );

    Ok(())
}

#[test]
fn add_index_usage_without_lock_needs_a_constraint() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_no_constraint", "1.2.3")?;
    out.assert().success();

    run_sysand_in(&cwd, ["add", "--no-lock", "acme/lib"], None)?
        .assert()
        .failure()
        .stderr(contains("an index usage needs a version constraint"));

    Ok(())
}

#[test]
fn add_rejects_an_invalid_publisher() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("g", "reject_add_shorthand", "1.2.3")?;
    out.assert().success();

    run_sysand_in(&cwd, ["add", "--no-lock", "A/My.Project", "^1"], None)?
        .assert()
        .failure()
        .stderr(contains("`A` is not a valid publisher"));

    Ok(())
}

/// Migrating a legacy PURL usage means removing it first
#[test]
fn add_index_usage_over_a_legacy_purl_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("g", "add_index_over_purl", "1.2.3")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "pkg:sysand/acme-labs/my-lib"],
        None,
    )?
    .assert()
    .success();
    for args in [
        &["add", "--no-lock", "Acme Labs/My Lib", "^1"][..],
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("h", "remove_index_usage", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "Acme Labs/My Lib", "^1"], None)?
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
        ["remove", "--no-lock", "pkg:sysand/acme-labs/my-lib"],
        None,
    )?
    .assert()
    .failure()
    .stderr(contains(
        "remove it with `sysand remove \"Acme Labs/My Lib\"`",
    ));

    run_sysand_in(&cwd, ["remove", "--no-lock", "Acme Labs/My Lib"], None)?
        .assert()
        .success()
        .stderr(contains(
            "Removed `Acme Labs/My Lib` with version constraints `^1`",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "remove_index_usage",
  "publisher": "h",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn remove_shorthand_removes_a_legacy_purl() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("h", "remove_shorthand_purl", "1.2.3")?;
    out.assert().success();

    run_sysand_in(
        &cwd,
        ["add", "--no-lock", "pkg:sysand/acme-labs/my.project"],
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["remove", "--no-lock", "Acme Labs/My.Project"], None)?
        .assert()
        .success()
        .stderr(contains("Removed `pkg:sysand/acme-labs/my.project`"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert!(
        !info_json.contains("pkg:sysand/acme-labs/my.project"),
        "{info_json}"
    );

    Ok(())
}

#[test]
fn remove_nonexistent_shorthand() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "remove_nonexistent_shorthand", "1.2.3")?;
    out.assert().success();

    run_sysand_in(&cwd, ["remove", "acme-labs/nonexistent"], None)?
        .assert()
        .failure()
        .stderr(contains("could not find usage for `acme-labs/nonexistent`"));

    Ok(())
}

#[test]
fn add_path_like_positional_suggests_path_option() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["add", "a/b/c"], None)?;

    out.assert()
        .failure()
        .stderr(contains("use `--path` instead"));

    Ok(())
}

#[test]
fn remove_path_like_positional_suggests_path_option() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["remove", "a/b/c"], None)?;

    out.assert()
        .failure()
        .stderr(contains("use `--path` instead"));

    Ok(())
}

/// Add and remove usages with `--path <path>`
#[test]
fn add_and_remove_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1, out1) = cli_init_project_basic("i", "add_and_remove_path1", "1.2.3")?;
    let (_temp_dir2, cwd2, out2) = cli_init_project_basic("i", "add_and_remove_path2", "1.2.3")?;
    let file_url = file_url_from_path(&cwd2);

    out1.assert().success();
    out2.assert().success();

    let out = run_sysand_in(&cwd1, ["add", "--no-lock", "--path", cwd2.as_str()], None)?;

    out.assert()
        .success()
        .stderr(contains(format!("Adding usage: IRI `{file_url}`")));

    let info_json = std::fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        format!(
            r#"{{
  "name": "add_and_remove_path1",
  "publisher": "i",
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

    let out = run_sysand_in(&cwd1, ["remove", "--path", cwd2.as_str()], None)?;

    out.assert().success().stderr(contains(format!(
        "Removing `{file_url}` from usages
     Removed `{file_url}`"
    )));

    let info_json = std::fs::read_to_string(cwd1.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_path1",
  "publisher": "i",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_and_remove_as_editable() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("j", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-editable",
            "local/test",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "j",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { editable = "local/test" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "j",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_as_local_src() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("k", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-local-src",
            "local/test",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "k",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { src_path = "local/test" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "k",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_as_local_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("l", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-local-kpar",
            "local/test.kpar",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "l",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { kpar_path = "local/test.kpar" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "l",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_as_remote_src() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("m", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-remote-src",
            "https://www.example.com/test",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "m",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { remote_src = "https://www.example.com/test" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "m",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_as_remote_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("n", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-remote-kpar",
            "https://www.example.com/test.kpar",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "n",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { remote_kpar = "https://www.example.com/test.kpar" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "n",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_as_remote_git() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("n", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test",
            "--as-remote-git",
            "https://www.example.com/test.git",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "n",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:test"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test",
]
sources = [
    { remote_git = "https://www.example.com/test.git" },
]
"#
    );

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "n",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

#[test]
fn add_and_remove_from_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("o", "add_and_remove", "1.2.3")?;

    out.assert().success();

    let config_path = cwd.join("sysand.toml");

    std::fs::create_dir_all(cwd.join("local/test"))?;

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test-src",
            "--from-path",
            "local/test",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:test-src` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test-src`"
    )));

    std::fs::File::create_new(cwd.join("local/test.kpar"))?;

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "urn:kpar:test-kpar",
            "--from-path",
            "local/test.kpar",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Adding source for `urn:kpar:test-kpar` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:test-kpar`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "o",
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

    let config = std::fs::read_to_string(&config_path)?;

    assert_eq!(
        config,
        r#"[[project]]
identifiers = [
    "urn:kpar:test-src",
]
sources = [
    { src_path = "local/test" },
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
        ["remove", "urn:kpar:test-src", "--no-lock"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:test-src` from usages
     Removed `urn:kpar:test-src`
    Removing source for `urn:kpar:test-src` from configuration file at `{config_path}`"
    )));

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:test-kpar", "--no-lock"],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Removing `urn:kpar:test-kpar` from usages
     Removed `urn:kpar:test-kpar`
    Removing source for `urn:kpar:test-kpar` from configuration file at `{config_path}`
    Removing empty configuration file at `{config_path}`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove",
  "publisher": "o",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

/// Add and remove a usage with `--from-url <file://...>`.
///
/// `--from-url` auto-resolves the URL and writes a `src_path` source into the
/// configuration file, similar to `--path` but driven by the URL resolver.
#[test]
fn add_and_remove_from_url() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir_dep, cwd_dep, out) = cli_init_project_basic("p", "add_from_url_dep", "1.2.3")?;
    out.assert().success();

    let (_temp_dir, cwd, out) = cli_init_project_basic("p", "add_from_url", "1.2.3")?;
    out.assert().success();

    let dep_url = file_url_from_path(&cwd_dep);
    let config_path = cwd.join("sysand.toml");

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
            "--from-url",
            &dep_url,
            "urn:kpar:add-from-url-dep",
        ],
        Some(config_path.as_str()),
    )?;

    out.assert().success().stderr(contains(format!(
        "Creating configuration file at `{config_path}`
      Adding source for `urn:kpar:add-from-url-dep` to configuration file at `{config_path}`
      Adding usage: IRI `urn:kpar:add-from-url-dep`"
    )));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "add_from_url",
  "publisher": "p",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:add-from-url-dep"
    }
  ]
}
"#
    );

    let config = std::fs::read_to_string(&config_path)?;
    assert!(
        config.contains("src_path"),
        "config should record a src_path source resolved from the file:// URL"
    );
    assert!(config.contains("urn:kpar:add-from-url-dep"));

    let out = run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:add-from-url-dep"],
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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "add_from_url",
  "publisher": "p",
  "version": "1.2.3"
}
"#
    );

    assert!(!config_path.is_file());

    Ok(())
}

/// `add <URL>` of a kpar served over HTTP installs it into `.sysand`
#[test]
fn add_from_http_kpar() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("t", "add_from_http_kpar", "1.0.0")?;
    out.assert().success();

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

    let out = run_sysand_in(&cwd, ["add", &project_url, "--no-index"], None)?;

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

/// Passing the full `pkg:sysand/publisher/name` PURL form directly must not
/// cause double-expansion. The scheme's colon prevents it from matching the
/// `publisher/name` shorthand heuristic, so the value is stored verbatim.
#[test]
fn add_and_remove_full_purl_sysand_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("q", "add_full_purl", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["add", "--no-lock", "pkg:sysand/acme-labs/my.project"],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Adding usage: IRI `pkg:sysand/acme-labs/my.project`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_full_purl",
  "publisher": "q",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "pkg:sysand/acme-labs/my.project"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["remove", "pkg:sysand/acme-labs/my.project"], None)?;

    out.assert().success().stderr(contains(
        "Removing `pkg:sysand/acme-labs/my.project` from usages
     Removed `pkg:sysand/acme-labs/my.project`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_full_purl",
  "publisher": "q",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

/// A `urn:` IRI whose path segment contains a slash has exactly two slash-separated
/// parts (`urn:kpar:acme-labs` and `my.project`), making it superficially resemble
/// `publisher/name` shorthand. The colon in the scheme must prevent any shorthand
/// expansion so the IRI is stored and removed verbatim.
#[test]
fn add_and_remove_urn_with_slash_not_treated_as_shorthand() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd, out) = cli_init_project_basic("r", "urn_slash", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["add", "--no-lock", "urn:kpar:acme-labs/my.project"],
        None,
    )?;

    out.assert().success().stderr(contains(
        "Adding usage: IRI `urn:kpar:acme-labs/my.project`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "urn_slash",
  "publisher": "r",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:acme-labs/my.project"
    }
  ]
}
"#
    );

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:acme-labs/my.project"], None)?;

    out.assert().success().stderr(contains(
        "Removing `urn:kpar:acme-labs/my.project` from usages
     Removed `urn:kpar:acme-labs/my.project`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "urn_slash",
  "publisher": "r",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

// The `env install` subcommand is commented out (see `EnvCommand` in cli.rs). Without the
// preinstall step this test would duplicate `add_and_remove_as_local_src` and
// `remove_keeps_lockfile_valid_and_syncs`, so it is commented out to match instead.
/*
#[test]
fn add_and_remove_with_lock_preinstall() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir_dep, cwd_dep, out) =
        cli_init_project_basic("a", "add_and_remove_with_lock_preinstall_dep", "1.2.3")?;

    out.assert().success();

    std::fs::write(
        cwd_dep.join("add_and_remove_with_lock_preinstall_dep.sysml"),
        "package AddAndRemoveWithLockLocalDep;",
    )?;

    run_sysand_in(
        &cwd_dep,
        ["include", "add_and_remove_with_lock_preinstall_dep.sysml"],
        None,
    )?
    .assert()
    .success();

    let (_temp_dir, cwd, out) =
        cli_init_project_basic("t", "add_and_remove_with_lock_preinstall", "1.2.3")?;

    out.assert().success();

    run_sysand_in(
        &cwd,
        [
            "env",
            "install",
            "urn:kpar:add_and_remove_with_lock_preinstall_dep",
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
            "urn:kpar:add_and_remove_with_lock_preinstall_dep",
            "--no-index",
        ],
        None,
    )?
    .assert()
    .success()
    .stderr(contains(
        "Adding usage: IRI `urn:kpar:add_and_remove_with_lock_preinstall_dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_with_lock_preinstall",
  "publisher": "t",
  "version": "1.2.3",
  "usage": [
    {
      "resource": "urn:kpar:add_and_remove_with_lock_preinstall_dep"
    }
  ]
}
"#
    );

    run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:add_and_remove_with_lock_preinstall_dep"],
        None,
    )?
    .assert()
    .success();

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "add_and_remove_with_lock_preinstall",
  "publisher": "t",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}
*/

#[test]
fn add_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_nonexistent", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["add", "urn:kpar:add_nonexistent"], None)?;

    out.assert()
        .failure()
        .stderr(contains("failed to retrieve project(s)"));

    Ok(())
}

#[test]
fn remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_nonexistent", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["remove", "urn:kpar:remove_nonexistent"], None)?;

    out.assert().failure().stderr(contains(
        "could not find usage for `urn:kpar:remove_nonexistent`",
    ));

    Ok(())
}

/// `add --no-sync` must update the lockfile but must not touch `.sysand` at
/// all (no env created, no install performed).
#[test]
fn add_no_sync_skips_env_sync() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_no_sync_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_dep, cwd_dep, out) = cli_init_project_basic("a", "add_no_sync_dep", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    let out = run_sysand_in(
        &cwd,
        [
            "add",
            "--no-sync",
            "urn:kpar:add-no-sync-dep",
            "--as-local-src",
            cwd_dep.as_str(),
        ],
        cfg,
    )?;

    out.assert()
        .success()
        .stderr(contains("Adding usage: IRI `urn:kpar:add-no-sync-dep`"))
        .stderr(predicate::str::contains("Syncing").not())
        .stderr(predicate::str::contains("Creating env").not());

    let lockfile =
        fs::read_to_string(cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME))?;
    assert!(
        lockfile.contains("add-no-sync-dep"),
        "lockfile must still be generated by `add --no-sync`: {lockfile}"
    );

    assert!(
        !cwd.join(DEFAULT_ENV_NAME).exists(),
        "`add --no-sync` must not create `.sysand`"
    );

    Ok(())
}

/// `add` must remove a project from `.sysand` once it is no longer present
/// in the freshly regenerated lockfile, while leaving dependencies that are
/// still needed (and the dependency being added) alone.
#[test]
fn add_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "add_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "add_prune_drop", "1.0.0")?;
    out.assert().success();

    let (_tmp_new, cwd_new, out) = cli_init_project_basic("a", "add_prune_new", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
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
    assert!(env_lib.join("kpar.add-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.add-prune-drop_1.0.0").is_dir());

    // Drop the usage and regenerate the lockfile without touching the env.
    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "urn:kpar:add-prune-drop"],
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
            "urn:kpar:add-prune-new",
            "--as-local-src",
            cwd_new.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    assert!(
        env_lib.join("kpar.add-prune-keep_1.0.0").is_dir(),
        "still-needed dependency must not be pruned"
    );
    assert!(
        env_lib.join("kpar.add-prune-new_1.0.0").is_dir(),
        "newly added dependency must be installed"
    );
    assert!(
        !env_lib.join("kpar.add-prune-drop_1.0.0").exists(),
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_no_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "add_no_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "add_no_prune_drop", "1.0.0")?;
    out.assert().success();

    let (_tmp_new, cwd_new, out) = cli_init_project_basic("a", "add_no_prune_new", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
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
    assert!(env_lib.join("kpar.add-no-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.add-no-prune-drop_1.0.0").is_dir());

    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "urn:kpar:add-no-prune-drop"],
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
            "urn:kpar:add-no-prune-new",
            "--as-local-src",
            cwd_new.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    assert!(env_lib.join("kpar.add-no-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.add-no-prune-new_1.0.0").is_dir());
    assert!(
        env_lib.join("kpar.add-no-prune-drop_1.0.0").is_dir(),
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_lock_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_dep, cwd_dep, out) = cli_init_project_basic("a", "remove_lock_dep", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
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
    assert!(env_lib.join("kpar.remove-lock-dep_1.0.0").is_dir());

    run_sysand_in(&cwd, ["remove", "urn:kpar:remove-lock-dep"], cfg)?
        .assert()
        .success();

    let lockfile =
        fs::read_to_string(cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME))?;
    assert!(
        !lockfile.contains("urn:kpar:remove-lock-dep"),
        "lockfile must not reference the removed dependency anywhere, including in the root's own usage list: {lockfile}"
    );

    assert!(
        !env_lib.join("kpar.remove-lock-dep_1.0.0").exists(),
        "the dependency dropped by `remove` must be pruned"
    );

    Ok(())
}

/// `remove` must remove a project from `.sysand` once it is no longer
/// needed, while leaving dependencies that are still needed alone.
#[test]
fn remove_prunes_unneeded_dependency_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "remove_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "remove_prune_drop", "1.0.0")?;
    out.assert().success();

    let (_tmp_extra, cwd_extra, out) = cli_init_project_basic("a", "remove_prune_extra", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
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
            "urn:kpar:remove-prune-extra",
            "--as-local-src",
            cwd_extra.as_str(),
        ],
        cfg,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("kpar.remove-prune-extra_1.0.0").is_dir());

    run_sysand_in(
        &cwd,
        ["remove", "urn:kpar:remove-prune-extra", "--no-prune"],
        cfg,
    )?
    .assert()
    .success();

    // Still physically present, but no longer part of the lockfile.
    assert!(env_lib.join("kpar.remove-prune-extra_1.0.0").is_dir());

    run_sysand_in(&cwd, ["remove", "urn:kpar:remove-prune-drop"], cfg)?
        .assert()
        .success();

    assert!(
        env_lib.join("kpar.remove-prune-keep_1.0.0").is_dir(),
        "still-needed dependency must be installed"
    );
    assert!(
        !env_lib.join("kpar.remove-prune-extra_1.0.0").exists(),
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_no_prune_app", "1.0.0")?;
    out.assert().success();

    let (_tmp_keep, cwd_keep, out) = cli_init_project_basic("a", "remove_no_prune_keep", "1.0.0")?;
    out.assert().success();

    let (_tmp_drop, cwd_drop, out) = cli_init_project_basic("a", "remove_no_prune_drop", "1.0.0")?;
    out.assert().success();

    let (_tmp_extra, cwd_extra, out) =
        cli_init_project_basic("a", "remove_no_prune_extra", "1.0.0")?;
    out.assert().success();

    let config_path = cwd.join("sysand.toml");
    let cfg = Some(config_path.as_str());

    run_sysand_in(
        &cwd,
        [
            "add",
            "--no-lock",
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
        ["remove", "urn:kpar:remove-no-prune-extra", "--no-prune"],
        cfg,
    )?
    .assert()
    .success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");

    run_sysand_in(
        &cwd,
        ["remove", "--no-prune", "urn:kpar:remove-no-prune-drop"],
        cfg,
    )?
    .assert()
    .success();

    assert!(
        env_lib.join("kpar.remove-no-prune-keep_1.0.0").is_dir(),
        "`--no-prune` must not skip syncing dependencies that are still needed"
    );
    assert!(
        env_lib.join("kpar.remove-no-prune-extra_1.0.0").is_dir(),
        "`--no-prune` must leave a project not present in the lockfile installed in `.sysand`"
    );

    Ok(())
}

/// With no constraint, `add` writes `^` the version that locking chose, here
/// a prerelease offered by a source override. The override is keyed by the
/// project's identifier, not the usage's spelling
#[test]
fn add_index_usage_constrains_to_the_locked_version() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_index_locked", "1.2.3")?;
    out.assert().success();
    cli_init_project_in(
        &cwd,
        Some("dep"),
        "Acme Labs",
        Some("My Lib"),
        Some("0.1.0-dev"),
        None,
    )?
    .assert()
    .success();
    let config_path = cwd.join("sysand.toml");

    run_sysand_in(
        &cwd,
        ["add", "--no-sync", "Acme Labs/My Lib", "--from-path", "dep"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success()
    .stderr(contains("Adding usage: `Acme Labs/My Lib` (^0.1.0-dev)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert!(
        info_json.contains(
            r#"{
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^0.1.0-dev"
    }"#
        ),
        "{info_json}"
    );
    let config = std::fs::read_to_string(&config_path)?;
    assert!(
        config.contains(r#""pkg:sysand/acme-labs/my-lib","#),
        "{config}"
    );
    let lock = std::fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "0.1.0-dev""#), "{lock}");

    run_sysand_in(
        &cwd,
        ["remove", "--no-lock", "Acme Labs/My Lib"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success();
    assert!(!config_path.is_file());

    Ok(())
}

/// A spelling that differs from the project's own is caught by locking, and
/// the manifest is restored
#[test]
fn add_index_usage_spelled_unlike_the_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_index_misspelled", "1.2.3")?;
    out.assert().success();
    cli_init_project_in(
        &cwd,
        Some("dep"),
        "Acme Labs",
        Some("My Lib"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();
    let before = std::fs::read_to_string(cwd.join(".project.json"))?;

    for args in [
        &["add", "--no-sync", "Acme labs/My Lib", "--from-path", "dep"][..],
        &[
            "add",
            "--no-sync",
            "Acme labs/My Lib",
            "^1",
            "--from-path",
            "dep",
        ][..],
    ] {
        run_sysand_in(
            &cwd,
            args.iter().copied(),
            Some(cwd.join("sysand.toml").as_str()),
        )?
        .assert()
        .failure()
        .stderr(contains(
            "index usage `Acme labs/My Lib` in `add_index_misspelled` 1.2.3 resolved to \
             version 1.0.0 of `Acme Labs/My Lib`, but is rejected because its spelling does \
             not match the project's;\nspell the usage exactly as `Acme Labs/My Lib`",
        ));
        assert_eq!(std::fs::read_to_string(cwd.join(".project.json"))?, before);
    }

    Ok(())
}

/// A normalized spelling takes the spelling of the project that locking
/// chooses, with or without a version constraint
#[test]
fn add_normalized_index_usage_takes_the_locked_spelling() -> Result<(), Box<dyn std::error::Error>>
{
    for (constraint, written) in [(Some("^1"), "^1"), (None, "^1.0.0")] {
        let (_temp_dir, cwd, out) =
            cli_init_project_basic("main", "add_index_normalized", "1.2.3")?;
        out.assert().success();
        cli_init_project_in(
            &cwd,
            Some("dep"),
            "Acme Labs",
            Some("My Lib"),
            Some("1.0.0"),
            None,
        )?
        .assert()
        .success();

        let mut args = vec!["add", "--no-sync", "acme-labs/my-lib"];
        args.extend(constraint);
        args.extend(["--from-path", "dep"]);
        run_sysand_in(&cwd, args, Some(cwd.join("sysand.toml").as_str()))?
            .assert()
            .success()
            .stderr(contains(format!(
                "Adding usage: `Acme Labs/My Lib` ({written})"
            )));

        let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
        assert!(
            info_json.contains(&format!(
                r#"{{
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "{written}"
    }}"#
            )),
            "{info_json}"
        );
        assert!(!info_json.contains("acme-labs"), "{info_json}");
    }

    Ok(())
}

/// Without locking, a normalized spelling takes the spelling of the matching
/// versions installed in the local environment
#[test]
fn add_normalized_index_usage_without_lock_takes_the_installed_spelling()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_env_spelling", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    // Does not match the constraint, so its spelling does not count
    install_in_env(&cwd, "ACME Labs", "My Lib", "2.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "acme-labs/my-lib", "^1"], None)?
        .assert()
        .success()
        .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert!(
        info_json.contains(
            r#"{
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^1"
    }"#
        ),
        "{info_json}"
    );

    // Adding it again the normalized way takes the declared spelling
    run_sysand_in(&cwd, ["add", "--no-lock", "acme-labs/my-lib", "^1"], None)?
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(cwd.join(".project.json"))?,
        info_json
    );

    Ok(())
}

/// Without locking, a spelling that cannot be checked against, or recovered
/// from, the local environment is refused
#[test]
fn add_index_usage_without_lock_needs_it_installed() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_not_installed", "1.2.3")?;
    out.assert().success();
    let before = std::fs::read_to_string(cwd.join(".project.json"))?;

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
        run_sysand_in(&cwd, ["add", "--no-lock", spelling, "^1"], None)?
            .assert()
            .failure()
            .stderr(contains(format!(
                "{message}: no version matching `^1` is installed in the local environment"
            )))
            .stderr(contains("leave out `--no-lock`"));
    }

    // Installed, but not in a version the constraint accepts
    install_in_env(&cwd, "Acme Labs", "My Lib", "2.0.0")?;
    run_sysand_in(&cwd, ["add", "--no-lock", "Acme Labs/My Lib", "^1"], None)?
        .assert()
        .failure()
        .stderr(contains("no version matching `^1` is installed"));

    assert_eq!(std::fs::read_to_string(cwd.join(".project.json"))?, before);

    Ok(())
}

/// Without locking, matching installed versions that disagree on the
/// spelling leave nothing to check against
#[test]
fn add_index_usage_without_lock_refuses_inconsistent_spellings()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_inconsistent", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    install_in_env(&cwd, "ACME Labs", "My Lib", "1.1.0")?;

    for spelling in ["Acme Labs/My Lib", "acme-labs/my-lib"] {
        run_sysand_in(&cwd, ["add", "--no-lock", spelling, "^1"], None)?
            .assert()
            .failure()
            .stderr(contains(format!(
                "versions of `{spelling}` installed in the local environment spell it \
                 differently: `ACME Labs/My Lib` (1.1.0), `Acme Labs/My Lib` (1.0.0)"
            )));
    }

    Ok(())
}
