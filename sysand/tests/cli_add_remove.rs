// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{fs, io::Write as _, str::FromStr as _};

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

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--iri", "urn:kpar:test"], None)?;

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

    let out = run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:test"], None)?;

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
fn add_rejects_sysand_shorthand() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "reject_add_shorthand", "1.2.3")?;

    out.assert().success();

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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "reject_add_shorthand",
  "publisher": "f",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_path_like_iri_suggests_path_options() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["add", "--iri", "a/b/c"], None)?;

    out.assert().failure().stderr(contains(
        "use `--dir`, `--kpar-path` or `--iri-path` instead",
    ));

    Ok(())
}

#[test]
fn remove_rejects_sysand_shorthand() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("h", "reject_remove_shorthand", "1.2.3")?;

    out.assert().success();

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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;

    assert_eq!(
        info_json,
        r#"{
  "name": "reject_remove_shorthand",
  "publisher": "h",
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
    let (_temp_dir, _cwd, out) = run_sysand(["remove", "--iri", "a/b/c"], None)?;

    out.assert()
        .failure()
        .stderr(contains("use `--iri-path` instead"));

    Ok(())
}

/// Add and remove usages with `--iri-path <path>`
#[test]
fn add_and_remove_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir1, cwd1, out1) = cli_init_project_basic("i", "add_and_remove_path1", "1.2.3")?;
    let (_temp_dir2, cwd2, out2) = cli_init_project_basic("i", "add_and_remove_path2", "1.2.3")?;
    let file_url = file_url_from_path(&cwd2);

    out1.assert().success();
    out2.assert().success();

    let out = run_sysand_in(
        &cwd1,
        ["add", "--no-lock", "--iri-path", cwd2.as_str()],
        None,
    )?;

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

    let out = run_sysand_in(&cwd1, ["remove", "--iri-path", cwd2.as_str()], None)?;

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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
            "--iri",
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
    let (_temp_dir, cwd, out) = cli_init_project_basic("q", "add_full_purl", "1.2.3")?;

    out.assert().success();

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

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "pkg:sysand/acme-labs/my.project"],
        None,
    )?;

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

/// A `urn:` IRI whose path segment contains a slash is stored and removed
/// verbatim.
#[test]
fn add_and_remove_urn_with_slash() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("r", "urn_slash", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["add", "--no-lock", "--iri", "urn:kpar:acme-labs/my.project"],
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

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:acme-labs/my.project"],
        None,
    )?;

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
            "--iri", "urn:kpar:add_and_remove_with_lock_preinstall_dep",
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
        ["remove", "--iri", "urn:kpar:add_and_remove_with_lock_preinstall_dep"],
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

    let out = run_sysand_in(&cwd, ["add", "--iri", "urn:kpar:add_nonexistent"], None)?;

    out.assert()
        .failure()
        .stderr(contains("failed to retrieve project(s)"));

    Ok(())
}

#[test]
fn remove_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_nonexistent", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "urn:kpar:remove_nonexistent"],
        None,
    )?;

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
    assert!(env_lib.join("kpar.add-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.add-prune-drop_1.0.0").is_dir());

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
    assert!(env_lib.join("kpar.add-no-prune-keep_1.0.0").is_dir());
    assert!(env_lib.join("kpar.add-no-prune-drop_1.0.0").is_dir());

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
    assert!(env_lib.join("kpar.remove-lock-dep_1.0.0").is_dir());

    run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:remove-lock-dep"], cfg)?
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
    assert!(env_lib.join("kpar.remove-prune-extra_1.0.0").is_dir());

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
    assert!(env_lib.join("kpar.remove-prune-extra_1.0.0").is_dir());

    run_sysand_in(&cwd, ["remove", "--iri", "urn:kpar:remove-prune-drop"], cfg)?
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
        env_lib.join("kpar.remove-no-prune-keep_1.0.0").is_dir(),
        "`--no-prune` must not skip syncing dependencies that are still needed"
    );
    assert!(
        env_lib.join("kpar.remove-no-prune-extra_1.0.0").is_dir(),
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
    let file = std::fs::File::create(kpar_path)?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);

    zip.start_file(".project.json", options)?;
    zip.write_all(
        format!(r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}"}}"#)
            .as_bytes(),
    )?;
    zip.start_file(".meta.json", options)?;
    zip.write_all(br#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)?;

    zip.finish()?;
    Ok(())
}

#[test]
fn add_dir_and_remove_identifier_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_and_remove", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "Acme Labs",
        Some("My Dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `Acme Labs/My Dep` from `dep`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "a",
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

    let out = run_sysand_in(&cwd, ["remove", "Acme Labs/My Dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `Acme Labs/My Dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `Acme Labs/My Dep` (path `dep`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_dir_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_no_publisher", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    std::fs::write(
        dep_dir.join(".project.json"),
        r#"{
  "name": "no-publisher-dep",
  "version": "1.0.0"
}
"#,
    )?;

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("does not have a publisher"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_no_publisher",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_dir_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_nonexistent", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    // dep exists as a directory but has no .project.json

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));

    Ok(())
}

#[test]
fn add_dir_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_add_already_present", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "Acme Labs",
        Some("My Dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("is already present"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_already_present",
  "publisher": "a",
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

    Ok(())
}

#[test]
fn remove_identifier() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "Acme Labs",
        Some("My Dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["remove", "Acme Labs/my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `Acme Labs/my-dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `Acme Labs/My Dep` (path `dep`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_remove",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn remove_identifier_nonexistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_nonexistent", "1.2.3")?;
    out.assert().success();

    let out = run_sysand_in(&cwd, ["remove", "Acme Labs/Nonexistent"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "could not find usage for `Acme Labs/Nonexistent`",
    ));

    Ok(())
}

#[test]
fn add_kpar_path_and_remove_identifier_without_lock() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_and_remove_kpar_path", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

    out.assert().success().stderr(predicate::str::contains(
        "Adding usage: `Acme Labs/My Dep` in `dep.kpar`",
    ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove_kpar_path",
  "publisher": "a",
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

    let out = run_sysand_in(&cwd, ["remove", "Acme Labs/My Dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `Acme Labs/My Dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `Acme Labs/My Dep` (path `dep.kpar`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_and_remove_kpar_path",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_kpar_path_missing_publisher_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_no_publisher", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    {
        let file = std::fs::File::create(&dep_kpar)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o644);
        zip.start_file(".project.json", options)?;
        zip.write_all(br#"{"name":"no-publisher-dep","version":"1.0.0"}"#)?;
        zip.finish()?;
    }

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("does not have a publisher"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_kpar_path_no_publisher",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn add_kpar_path_nonexistent_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_nonexistent", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    // dep.kpar exists as a valid, but empty, archive: no `.project.json`
    {
        let file = std::fs::File::create(&dep_kpar)?;
        zip::ZipWriter::new(file).finish()?;
    }

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "unable to find interchange project",
    ));

    Ok(())
}

#[test]
fn add_kpar_path_already_present_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "exp_add_kpar_path_already_present", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains("is already present"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_add_kpar_path_already_present",
  "publisher": "a",
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

    Ok(())
}

#[test]
fn remove_identifier_kpar_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_kpar_path", "1.2.3")?;
    out.assert().success();

    let dep_kpar = cwd.join("dep.kpar");
    write_dep_kpar(&dep_kpar, "Acme Labs", "My Dep", "1.0.0")?;

    run_sysand_in(&cwd, ["add", "--no-lock", "--kpar-path", "dep.kpar"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["remove", "acme-labs/my-dep"], None)?;

    out.assert()
        .success()
        .stderr(predicate::str::contains(
            "Removing `acme-labs/my-dep` from usages",
        ))
        .stderr(predicate::str::contains(
            "Removed `Acme Labs/My Dep` (path `dep.kpar`)",
        ));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "exp_remove_kpar_path",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

/// After `remove <publisher>/<name>` updates an existing lockfile, the lockfile
/// must remain internally consistent, and the dependency should be gone
/// from `.sysand`.
#[test]
fn remove_identifier_keeps_lockfile_valid_and_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "exp_remove_lock_app", "1.0.0")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "Acme Labs",
        Some("Remove Lock Dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    run_sysand_in(&cwd, ["lock"], None)?.assert().success();
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();

    let env_lib = cwd.join(DEFAULT_ENV_NAME).join("lib");
    assert!(env_lib.join("acme-labs-remove-lock-dep_1.0.0").is_dir());

    run_sysand_in(&cwd, ["remove", "Acme Labs/Remove Lock Dep"], None)?
        .assert()
        .success();

    let lockfile =
        fs::read_to_string(cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME))?;
    assert!(
        !lockfile.contains("Remove Lock Dep") && !lockfile.contains("remove-lock-dep"),
        "lockfile must not reference the removed dependency anywhere, including in the root's own usage list: {lockfile}"
    );

    assert!(
        !env_lib.join("acme-labs-remove-lock-dep_1.0.0").exists(),
        "the dependency dropped by `remove <publisher>/<name>` must be pruned"
    );

    Ok(())
}

#[test]
fn remove_identifier_removes_the_sysand_purl_resource() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_identifier_purl", "1.2.3")?;
    out.assert().success();

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

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "remove_identifier_purl",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn remove_identifier_matches_a_typed_usage_by_the_normalized_form()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "remove_identifier_normalized", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "Acme Labs",
        Some("My Dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    let out = run_sysand_in(&cwd, ["remove", "--no-lock", "acme-labs/my-dep"], None)?;

    out.assert()
        .success()
        .stderr(contains("Removed `Acme Labs/My Dep` (path `dep`)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert_eq!(
        info_json,
        r#"{
  "name": "remove_identifier_normalized",
  "publisher": "a",
  "version": "1.2.3"
}
"#
    );

    Ok(())
}

#[test]
fn remove_iri_of_a_typed_usage_suggests_identifier() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_iri_of_typed", "1.2.3")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    // A valid `pkg:sysand` publisher, so the typed usage has a PURL identifier
    cli_init_project_in(
        &dep_dir,
        None,
        "acme-labs",
        Some("my-dep"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-lock", "--dir", "dep"], None)?
        .assert()
        .success();

    let out = run_sysand_in(
        &cwd,
        ["remove", "--iri", "pkg:sysand/acme-labs/my-dep"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(contains("declared as a directory usage"))
        .stderr(contains("sysand remove acme-labs/my-dep"));

    Ok(())
}

/// Removing a typed usage by its normalized spelling must also remove it
/// from the lockfile. The lock records the identifier derived from the
/// declared spelling, which differs from the one derived from the normalized
/// spelling when the result is not a valid `pkg:sysand` PURL (here the
/// publisher's `.`), so the identifier must come from the removed usage
#[test]
fn remove_typed_by_normalized_spelling_updates_lockfile() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_norm_app", "1.0.0")?;
    out.assert().success();

    let dep_dir = cwd.join("dep");
    std::fs::create_dir_all(&dep_dir)?;
    cli_init_project_in(
        &dep_dir,
        None,
        "ACME Inc.",
        Some("Foo"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();

    run_sysand_in(&cwd, ["add", "--no-index", "--dir", "dep"], None)?
        .assert()
        .success();

    let lockfile_path = cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);
    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert!(
        lockfile.contains("urn:sysand:ACME%20Inc./Foo"),
        "lockfile must reference the added dependency: {lockfile}"
    );

    run_sysand_in(&cwd, ["remove", "--no-index", "acme-inc./foo"], None)?
        .assert()
        .success()
        .stderr(contains("Removed `ACME Inc./Foo` (path `dep`)"));

    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert!(
        !lockfile.contains("urn:sysand:ACME%20Inc./Foo"),
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

/// A typed `sysand add <identifier>` should reject an invalid identifier
/// while parsing arguments, before adding anything
#[test]
fn add_identifier_rejects_invalid_identifiers() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_identifier_invalid", "1.2.3")?;
    out.assert().success();
    let original = fs::read_to_string(cwd.join(".project.json"))?;

    for &(identifier, msg) in INVALID_IDENTIFIERS {
        let out = run_sysand_in(&cwd, ["add", "--no-lock", identifier], None)?;

        out.assert().failure().stderr(
            contains(format!("invalid value '{identifier}' for '[IDENTIFIER]'")).and(contains(msg)),
        );
        assert_eq!(
            fs::read_to_string(cwd.join(".project.json"))?,
            original,
            "identifier: {identifier:?}"
        );
    }

    Ok(())
}

/// `sysand add <identifier>` should accept spaces in publisher and name,
/// reaching the typed add
#[test]
fn add_identifier_accepts_spaces() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "add_identifier_spaces", "1.2.3")?;
    out.assert().success();

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
    let (_temp_dir, cwd, out) = cli_init_project_basic("a", "remove_identifier_invalid", "1.2.3")?;
    out.assert().success();
    let original = fs::read_to_string(cwd.join(".project.json"))?;

    for &(identifier, msg) in INVALID_IDENTIFIERS {
        let out = run_sysand_in(&cwd, ["remove", "--no-lock", identifier], None)?;

        out.assert().failure().stderr(
            contains(format!("invalid value '{identifier}' for '[IDENTIFIER]'")).and(contains(msg)),
        );
        assert_eq!(
            fs::read_to_string(cwd.join(".project.json"))?,
            original,
            "identifier: {identifier:?}"
        );
    }

    Ok(())
}

/// Removing a usage whose project is still needed transitively must drop it
/// from the root's usages in the lockfile, even though no project is pruned
#[test]
fn remove_still_needed_dependency_updates_root_usages_in_lockfile()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("acme", "remove_needed_app", "1.0.0")?;
    out.assert().success();

    for name in ["dep-a", "dep-b"] {
        let dir = cwd.join(name);
        std::fs::create_dir_all(&dir)?;
        cli_init_project_in(&dir, None, "acme", Some(name), Some("1.0.0"), None)?
            .assert()
            .success();
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

    let lockfile_path = cwd.join(sysand_core::commands::lock::DEFAULT_LOCKFILE_NAME);
    let root_usages = |lockfile: &str| -> Vec<String> {
        let lock = sysand_core::lock::Lock::from_str(lockfile).unwrap();
        let root = lock
            .projects
            .iter()
            .find(|p| p.name == "remove_needed_app")
            .unwrap();
        root.usages.iter().map(|u| u.to_string()).collect()
    };
    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert_eq!(
        root_usages(&lockfile),
        ["pkg:sysand/acme/dep-a", "pkg:sysand/acme/dep-b"]
    );

    run_sysand_in(&cwd, ["remove", "--no-index", "acme/dep-b"], None)?
        .assert()
        .success();

    let lockfile = fs::read_to_string(&lockfile_path)?;
    assert_eq!(root_usages(&lockfile), ["pkg:sysand/acme/dep-a"]);
    assert!(
        lockfile.contains("name = \"dep-b\""),
        "`dep-b` is still needed by `dep-a`, so it must stay locked: {lockfile}"
    );

    Ok(())
}

#[test]
fn add_publisher_name_writes_an_index_usage() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("f", "add_index_usage", "1.2.3")?;
    out.assert().success();
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
        "index usage `A/My.Project` has an invalid publisher `A`",
    ));

    Ok(())
}

/// Migrating a legacy PURL usage means removing it first
#[test]
fn add_index_usage_over_a_legacy_purl_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("g", "add_index_over_purl", "1.2.3")?;
    out.assert().success();

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
    let (_temp_dir, cwd, out) = cli_init_project_basic("h", "remove_index_usage", "1.2.3")?;
    out.assert().success();
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
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("a", "remove_nonexistent_identifier", "1.2.3")?;

    out.assert().success();

    let out = run_sysand_in(&cwd, ["remove", "acme-labs/nonexistent"], None)?;

    out.assert()
        .failure()
        .stderr(contains("could not find usage for `acme-labs/nonexistent`"));

    Ok(())
}

/// With no constraint, `add` writes `^` the highest release it resolves,
/// here from the local environment. A prerelease is not a release
#[test]
fn add_index_usage_constrains_to_the_highest_release() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_index_highest", "1.2.3")?;
    out.assert().success();
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
    .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1.2.0)"));

    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    assert!(
        info_json.contains(
            r#"{
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^1.2.0"
    }"#
        ),
        "{info_json}"
    );
    let lock = std::fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "1.2.0""#), "{lock}");

    Ok(())
}

/// A spelling that differs from the project's own is caught by locking, and
/// the manifest is restored
#[test]
fn add_index_usage_spelled_unlike_the_project_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_index_misspelled", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    let before = std::fs::read_to_string(cwd.join(".project.json"))?;

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
                "index usage `Acme labs/My Lib` in `add_index_misspelled` 1.2.3 resolved to \
                 version 1.0.0 of `Acme Labs/My Lib`, but is rejected because its spelling does \
                 not match the project's;\nspell the usage exactly as `Acme Labs/My Lib`",
            ));
        assert_eq!(std::fs::read_to_string(cwd.join(".project.json"))?, before);
    }

    Ok(())
}

/// A normalized spelling takes the spelling of the project resolved, with or
/// without a version constraint
#[test]
fn add_normalized_index_usage_takes_the_resolved_spelling() -> Result<(), Box<dyn std::error::Error>>
{
    for (constraint, written) in [(Some("^1"), "^1"), (None, "^1.0.0")] {
        let (_temp_dir, cwd, out) =
            cli_init_project_basic("main", "add_index_normalized", "1.2.3")?;
        out.assert().success();
        install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;

        let mut args = vec!["add", "--no-sync", "--no-index", "acme-labs/my-lib"];
        if let Some(constraint) = constraint {
            args.extend(["--version-constraint", constraint]);
        }
        run_sysand_in(&cwd, args, None)?
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

/// Source overrides are only for resource usages
#[test]
fn add_index_usage_refuses_a_source_override() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_index_override", "1.2.3")?;
    out.assert().success();

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
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "override_not_index", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    cli_init_project_in(
        &cwd,
        Some("dep"),
        "Acme Labs",
        Some("My Lib"),
        Some("9.0.0"),
        None,
    )?
    .assert()
    .success();
    let config_path = cwd.join("sysand.toml");
    std::fs::write(
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
    .stderr(contains("Adding usage: `Acme Labs/My Lib` (^1.0.0)"));

    let lock = std::fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "1.0.0""#), "{lock}");
    assert!(!lock.contains(r#"version = "9.0.0""#), "{lock}");

    Ok(())
}

/// Known issue: the solver coalesces usages by identifier, and the first one
/// resolved supplies the candidates for all of them (see the TODO in
/// `resolve_candidates`). Here the root's overridden resource usage is
/// resolved before its dependency's index usage of the same project, so the
/// index usage is answered by the override too. Update this test when the
/// coalescing policy is decided
#[test]
fn source_override_reaches_an_index_usage_of_the_same_project_through_coalescing()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "override_coalesced", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    cli_init_project_in(
        &cwd,
        Some("dep"),
        "Acme Labs",
        Some("My Lib"),
        Some("9.0.0"),
        None,
    )?
    .assert()
    .success();
    cli_init_project_in(
        &cwd,
        Some("mid"),
        "Acme Labs",
        Some("Mid"),
        Some("1.0.0"),
        None,
    )?
    .assert()
    .success();
    let mid = cwd.join("mid");
    // `^9` is satisfied only by the override
    let mid_info = std::fs::read_to_string(mid.join(".project.json"))?;
    let mid_info = mid_info.replacen(
        r#""version": "1.0.0""#,
        r#""version": "1.0.0",
  "usage": [
    {
      "publisher": "Acme Labs",
      "name": "My Lib",
      "versionConstraint": "^9"
    }
  ]"#,
        1,
    );
    std::fs::write(mid.join(".project.json"), mid_info)?;

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
    let lock = std::fs::read_to_string(cwd.join("sysand-lock.toml"))?;
    assert!(lock.contains(r#"version = "9.0.0""#), "{lock}");

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
        run_sysand_in(
            &cwd,
            ["add", "--no-lock", spelling, "--version-constraint", "^1"],
            None,
        )?
        .assert()
        .failure()
        .stderr(contains(format!(
            "{message}: no version matching `^1` is installed in the local environment"
        )))
        .stderr(contains("leave out `--no-lock`"));
    }

    // Installed, but not in a version the constraint accepts
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
        run_sysand_in(
            &cwd,
            ["add", "--no-lock", spelling, "--version-constraint", "^1"],
            None,
        )?
        .assert()
        .failure()
        .stderr(contains(format!(
            "versions of `{spelling}` installed in the local environment spell it \
                 differently: `ACME Labs/My Lib` (1.1.0), `Acme Labs/My Lib` (1.0.0)"
        )));
    }

    Ok(())
}

/// Adding a usage that is already present still locks and syncs, since the
/// environment may be missing or stale
#[test]
fn add_already_present_usage_still_syncs() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_present_syncs", "1.2.3")?;
    out.assert().success();
    cli_init_project_in(&cwd, Some("dep"), "Acme", Some("Dep"), Some("1.0.0"), None)?
        .assert()
        .success();
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
    assert!(env.is_dir());
    std::fs::remove_dir_all(&env)?;

    run_sysand_in(
        &cwd,
        ["add", "--no-index", "--iri", "urn:kpar:dep"],
        Some(config_path.as_str()),
    )?
    .assert()
    .success()
    .stderr(contains("since it is already present"));

    assert!(env.is_dir(), "the environment must be synced again");

    Ok(())
}

/// Adding an index usage that is already present, without a constraint,
/// leaves it as it is but still locks
#[test]
fn add_already_present_index_usage_still_locks() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("main", "add_present_locks", "1.2.3")?;
    out.assert().success();
    install_in_env(&cwd, "Acme Labs", "My Lib", "1.0.0")?;
    run_sysand_in(
        &cwd,
        ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
        None,
    )?
    .assert()
    .success();
    let info_json = std::fs::read_to_string(cwd.join(".project.json"))?;
    let lock_path = cwd.join("sysand-lock.toml");
    std::fs::remove_file(&lock_path)?;

    run_sysand_in(
        &cwd,
        ["add", "--no-sync", "--no-index", "Acme Labs/My Lib"],
        None,
    )?
    .assert()
    .success()
    .stderr(contains("since it is already present"));

    assert_eq!(
        std::fs::read_to_string(cwd.join(".project.json"))?,
        info_json
    );
    assert!(lock_path.is_file(), "the lockfile must be written again");

    Ok(())
}
