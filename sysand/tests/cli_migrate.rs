// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand migrate`.

// Each test binary compiles its own copy of `common`, and this one
// uses a small part of it.
#[expect(dead_code)]
mod common;

use assert_cmd::prelude::*;
use camino::Utf8Path;
use predicates::prelude::*;

use common::{git_commit_files, new_temp_cwd, run_sysand_in, write_legacy_project};

/// A git repository around `dir`, with everything in it committed, or
/// with an empty commit when `commit` is false.
fn git_repo(dir: &Utf8Path, commit: bool) -> Result<(), Box<dyn std::error::Error>> {
    fn collect(
        dir: &Utf8Path,
        prefix: &str,
        files: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for entry in dir.read_dir_utf8()? {
            let entry = entry?;
            let path = format!("{prefix}{}", entry.file_name());
            if entry.file_type()?.is_dir() {
                collect(entry.path(), &format!("{path}/"), files)?;
            } else {
                files.push((path, std::fs::read(entry.path())?));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    if commit {
        collect(dir, "", &mut files)?;
    }
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_slice()))
        .collect();
    git_commit_files(dir, &files)
}

fn json_project(dir: &Utf8Path) -> Result<(), Box<dyn std::error::Error>> {
    write_legacy_project(dir, "acme", "widgets", "1.2.3", None)?;
    std::fs::write(dir.join("test.sysml"), b"package P;\n")?;
    Ok(())
}

#[test]
fn migrate_converts_a_json_project() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_project(&cwd)?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().success();

    assert!(cwd.join("model.toml").is_file(), "the manifest is written");
    assert!(!cwd.join(".project.json").exists(), "the json pair is gone");
    assert!(!cwd.join(".meta.json").exists(), "the json pair is gone");
    let text = std::fs::read_to_string(cwd.join("model.toml"))?;
    assert!(text.contains("name = \"widgets\""), "{text}");
    assert!(text.contains("publisher = \"acme\""), "{text}");
    Ok(())
}

/// The round trip: what `migrate` writes, sysand reads.
#[test]
fn a_migrated_project_reads_back() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_project(&cwd)?;
    git_repo(&cwd, true)?;
    run_sysand_in(&cwd, ["migrate"], None)?.assert().success();

    run_sysand_in(&cwd, ["info"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: widgets"))
        .stdout(predicate::str::contains("Version: 1.2.3"));
    Ok(())
}

/// The check runs *before* anything is written.
#[test]
fn migrate_refuses_a_dirty_tree() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_project(&cwd)?;
    git_repo(&cwd, true)?;
    std::fs::write(cwd.join("test.sysml"), b"package Q;\n")?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"))
        .stderr(predicate::str::contains("--allow-dirty"));
    assert!(
        !cwd.join("model.toml").exists(),
        "the check runs before the write"
    );
    assert!(cwd.join(".project.json").exists(), "nothing was deleted");

    run_sysand_in(&cwd, ["migrate", "--allow-dirty"], None)?
        .assert()
        .success();
    assert!(cwd.join("model.toml").is_file());
    Ok(())
}

#[test]
fn migrate_refuses_a_directory_with_no_repository() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_project(&cwd)?;

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_sysand"));
    command
        .arg("migrate")
        .current_dir(&cwd)
        // `discover` walks upward, and the temporary directory may sit
        // inside a work tree of its own.
        .env("GIT_CEILING_DIRECTORIES", cwd.as_str());
    let out = command.output()?;
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not inside a git work tree"), "{stderr}");
    assert!(stderr.contains("--allow-no-vcs"), "{stderr}");
    assert!(!cwd.join("model.toml").exists(), "nothing was written");

    run_sysand_in(&cwd, ["migrate", "--allow-no-vcs"], None)?
        .assert()
        .success();
    assert!(cwd.join("model.toml").is_file());
    Ok(())
}

/// model-toml.md "JSON to TOML": from a workspace root the command
/// errors unless a path names the project.
#[test]
fn migrate_from_a_workspace_root_needs_a_path() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n",
    )?;
    let app = cwd.join("app");
    std::fs::create_dir(&app)?;
    json_project(&app)?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "run from a workspace root, `migrate` needs the path",
        ));

    run_sysand_in(&cwd, ["migrate", "app"], None)?
        .assert()
        .success();
    assert!(app.join("model.toml").is_file());
    Ok(())
}

#[test]
fn migrate_refuses_a_toml_project() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\n\
         version = \"1.0.0\"\nsources = []\n",
    )?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("already a `model.toml` project"));
    Ok(())
}

/// `[project]` requires a publisher, and none was given.
#[test]
fn migrate_without_a_publisher_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".project.json"),
        br#"{"name": "widgets", "version": "1.2.3", "usage": []}"#,
    )?;
    std::fs::write(
        cwd.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#,
    )?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("declares no `publisher`"));
    assert!(!cwd.join("model.toml").exists(), "nothing was written");

    run_sysand_in(&cwd, ["migrate", "--publisher", "acme"], None)?
        .assert()
        .success();
    assert!(cwd.join("model.toml").is_file());
    Ok(())
}

/// A conversion that fails leaves the project exactly as it was.
#[test]
fn nothing_is_written_when_the_conversion_fails() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join(".project.json"),
        br#"{"name": "widgets", "publisher": "acme", "version": "2023-11", "usage": []}"#,
    )?;
    std::fs::write(
        cwd.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#,
    )?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().failure();
    assert!(!cwd.join("model.toml").exists());
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives"
    );
    assert!(cwd.join(".meta.json").is_file(), "the json pair survives");
    Ok(())
}

/// The write happens before the delete: a write that fails must not
/// leave a project with no manifest at all.
#[test]
fn a_failed_write_keeps_the_json_pair() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_project(&cwd)?;
    // A directory where the manifest should go: the write cannot
    // succeed, and the deletion must not have happened.
    std::fs::create_dir(cwd.join("model.toml"))?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().failure();
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives a failed write"
    );
    assert!(cwd.join(".meta.json").is_file());
    Ok(())
}

// ----------------------------------- the manifests migrate may meet

fn json_with_usages(dir: &Utf8Path, usages: &str) -> Result<(), Box<dyn std::error::Error>> {
    // `write_legacy_project` writes no `usage` key at all.
    std::fs::write(
        dir.join(".project.json"),
        format!(
            r#"{{"name": "widgets", "publisher": "acme", "version": "1.2.3", "usage": [{usages}]}}"#
        ),
    )?;
    std::fs::write(
        dir.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#,
    )?;
    Ok(())
}

/// Succeeding here would delete the json pair and leave a `model.toml`
/// that `sysand` itself refuses to read.
#[test]
fn migrate_keeps_a_project_with_duplicate_usages() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_with_usages(
        &cwd,
        r#"{"dir": "../dep", "publisher": "acme", "name": "dep"},
           {"dir": "../other", "publisher": "Acme", "name": "Dep"}"#,
    )?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("name the same project"));
    assert!(!cwd.join("model.toml").exists(), "nothing was written");
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives"
    );
    assert!(cwd.join(".meta.json").is_file(), "the json pair survives");
    Ok(())
}

/// The round trip with usages: distinct usages survive and the result
/// reads back.
#[test]
fn a_migrated_project_reads_back_with_usages() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    json_with_usages(
        &cwd,
        r#"{"dir": "../alpha", "publisher": "acme", "name": "alpha"},
           {"dir": "../beta", "publisher": "acme", "name": "beta"}"#,
    )?;
    git_repo(&cwd, true)?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().success();
    run_sysand_in(&cwd, ["info"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: widgets"));
    let text = std::fs::read_to_string(cwd.join("model.toml"))?;
    assert!(text.contains("alpha"), "{text}");
    assert!(text.contains("beta"), "{text}");
    Ok(())
}

/// A `model.toml` beside a json pair: the toml is kept byte for byte
/// and only the json files go.
fn both_fixture(
    cwd: &Utf8Path,
    toml: &str,
    json_name: &str,
    json_version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::write(cwd.join("model.toml"), toml)?;
    std::fs::write(
        cwd.join(".project.json"),
        format!(
            r#"{{"name": "{json_name}", "publisher": "acme", "version": "{json_version}", "usage": []}}"#
        ),
    )?;
    std::fs::write(
        cwd.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#,
    )?;
    git_repo(cwd, true)
}

const AGREEING_TOML: &str = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\n\
     name = \"widgets\"\nversion = \"1.2.3\"\nsources = []\n";

#[test]
fn migrate_keeps_an_existing_model_toml() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    both_fixture(&cwd, AGREEING_TOML, "widgets", "1.2.3")?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().success();
    assert_eq!(
        std::fs::read_to_string(cwd.join("model.toml"))?,
        AGREEING_TOML,
        "the existing manifest is untouched"
    );
    assert!(!cwd.join(".project.json").exists());
    assert!(!cwd.join(".meta.json").exists());
    Ok(())
}

#[test]
fn migrate_refuses_a_disagreeing_pair() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    // The *version* alone differs, so a comparison that checks only the
    // name would let this through.
    both_fixture(&cwd, AGREEING_TOML, "widgets", "9.9.9")?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("1.2.3"))
        .stderr(predicate::str::contains("9.9.9"));
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives"
    );
    Ok(())
}

/// Identities normalize: `Lib test` and `lib-test` are one name.
#[test]
fn the_both_comparison_normalizes() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let toml = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\n\
                name = \"lib-test\"\nversion = \"1.2.3\"\nsources = []\n";
    both_fixture(&cwd, toml, "Lib test", "1.2.3")?;

    run_sysand_in(&cwd, ["migrate"], None)?.assert().success();
    assert!(!cwd.join(".project.json").exists());
    Ok(())
}

/// With an explicit path: a bare invocation at a workspace root is
/// refused for lacking one before the pair is looked at.
#[test]
fn migrate_refuses_a_workspace_beside_a_json_pair() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    both_fixture(
        &cwd,
        "manifest-version = 1\n\n[workspace]\nmembers = []\n",
        "widgets",
        "1.2.3",
    )?;

    run_sysand_in(&cwd, ["migrate", cwd.as_str()], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "defines a workspace, not a project",
        ));
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives"
    );
    Ok(())
}

#[test]
fn migrate_refuses_an_inherited_version() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let toml = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\n\
                name = \"widgets\"\nversion.workspace = true\nsources = []\n";
    both_fixture(&cwd, toml, "widgets", "1.2.3")?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("inherits `version`"));
    assert!(
        cwd.join(".project.json").is_file(),
        "the json pair survives"
    );
    Ok(())
}

/// The working-tree guard comes first: a dirty `Both` project with a
/// mismatch hears about the dirty tree, not the mismatch.
#[test]
fn the_vcs_check_precedes_the_both_comparison() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    both_fixture(&cwd, AGREEING_TOML, "widgets", "9.9.9")?;
    std::fs::write(
        cwd.join("model.toml"),
        format!("{AGREEING_TOML}# changed\n"),
    )?;

    run_sysand_in(&cwd, ["migrate"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
    Ok(())
}
