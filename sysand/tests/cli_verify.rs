// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand sync --verify` and `--repair`: the environment is read back
//! from disk, not trusted from `env.toml`.

use std::{error::Error, fs};

use assert_cmd::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    env::{DEFAULT_ENV_NAME, local_directory::METADATA_PATH},
    lock::{Lock, Lockfile, Project, Source, Usage},
    project::{ProjectRead as _, local_src::LocalSrcProject},
    utils::sha256_lowercase_hex,
};
use typed_path::Utf8UnixPathBuf;

mod common;
pub use common::*;

/// A dependency directory with one checksummed model file.
fn write_dep(dir: &Utf8Path, name: &str) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(dir)?;
    let model = format!("package {name} {{ }}\n");
    fs::write(dir.join(format!("{name}.sysml")), &model)?;
    fs::write(
        dir.join(".project.json"),
        format!(r#"{{"name":"{name}","publisher":"acme","version":"1.0.0"}}"#),
    )?;
    fs::write(
        dir.join(".meta.json"),
        format!(
            r#"{{"index":{{"{name}":"{name}.sysml"}},"created":"2026-01-01T00:00:00Z","checksum":{{"{name}.sysml":{{"algorithm":"SHA256","value":"{}"}}}}}}"#,
            sha256_lowercase_hex(&model)
        ),
    )?;
    Ok(())
}

fn write_app(dir: &Utf8Path, deps: &[&str], constraint: &str) -> Result<(), Box<dyn Error>> {
    let usages: Vec<String> = deps
        .iter()
        .map(|dep| format!(r#"{{"resource":"urn:{dep}","versionConstraint":"{constraint}"}}"#))
        .collect();
    fs::write(
        dir.join(".project.json"),
        format!(
            r#"{{"name":"app","publisher":"acme","version":"0.1.0","usage":[{}]}}"#,
            usages.join(",")
        ),
    )?;
    fs::write(
        dir.join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )?;
    Ok(())
}

/// An app in `cwd` using `deps` from `deps/<name>`, locked and synced.
fn synced_app(deps: &[&str]) -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn Error>> {
    let (temp_dir, cwd) = new_temp_cwd()?;
    write_app(&cwd, deps, "^1.0.0")?;
    let mut projects = vec![Project {
        publisher: Some("acme".to_owned()),
        name: "app".to_owned(),
        version: "0.1.0".to_owned(),
        metamodel: None,
        exports: vec![],
        identifiers: vec![],
        usages: deps
            .iter()
            .map(|dep| Usage::from_str_unchecked(&format!("urn:{dep}")))
            .collect(),
        sources: vec![Source::Editable {
            editable: Utf8UnixPathBuf::from("."),
        }],
    }];
    for dep in deps {
        let dir = cwd.join("deps").join(dep);
        write_dep(&dir, dep)?;
        let checksum = LocalSrcProject::new_access(&dir, None)
            .checksum_canonical_hex()?
            .expect("manifests");
        projects.push(Project {
            publisher: Some("acme".to_owned()),
            name: (*dep).to_owned(),
            version: "1.0.0".to_owned(),
            metamodel: None,
            exports: vec![],
            identifiers: vec![format!("urn:{dep}")],
            usages: vec![],
            sources: vec![Source::LocalSrc {
                src_path: Utf8UnixPathBuf::from(format!("deps/{dep}")),
                checksum,
            }],
        });
    }
    let lock = Lock {
        projects,
        ..Lock::default()
    };
    Lockfile::new(&cwd, lock).write()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    Ok((temp_dir, cwd))
}

/// Where `dep` is installed.
fn installed(cwd: &Utf8Path, dep: &str) -> Result<Utf8PathBuf, Box<dyn Error>> {
    let env_toml = fs::read_to_string(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    let path = env_toml
        .lines()
        .skip_while(|line| *line != format!("name = \"{dep}\""))
        .find_map(|line| line.strip_prefix("path = \""))
        .map(|rest| rest.trim_end_matches('"'))
        .expect("installed path");
    Ok(cwd.join(DEFAULT_ENV_NAME).join(path))
}

#[test]
fn verify_clean_is_a_no_op() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0", "dep1"])?;
    let env_before = fs::read(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?;
    let lock_before = fs::read(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("Verifying"))
        .stderr(predicate::str::contains(
            "`.sysand` matches `sysand-lock.toml`: 2 project(s) verified",
        ))
        .stderr(predicate::str::contains("Installing").not());
    assert_eq!(
        env_before,
        fs::read(cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH))?
    );
    assert_eq!(lock_before, fs::read(cwd.join(DEFAULT_LOCKFILE_NAME))?);
    Ok(())
}

#[test]
fn verify_reports_tampered_file() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0"])?;
    let file = installed(&cwd, "dep0")?.join("dep0.sysml");
    fs::write(&file, "package dep0 { part p; }\n")?;
    let tampered = fs::read(&file)?;
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "installed project `urn:dep0` does not match the lockfile checksum",
        ))
        .stderr(predicate::str::contains("`dep0.sysml` differs"))
        .stderr(predicate::str::contains(
            "`.sysand` does not match `sysand-lock.toml`: 1 problem(s)",
        ));
    // `--verify` writes nothing: the tampered bytes are still there.
    assert_eq!(tampered, fs::read(&file)?);
    Ok(())
}

#[test]
fn verify_reports_missing_directory() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0", "dep1"])?;
    fs::remove_dir_all(installed(&cwd, "dep1")?)?;
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert().failure().stderr(predicate::str::contains(
        "`env.toml` records `urn:dep1`, which is not installed",
    ));
    // A plain `sync` trusts `env.toml` and sees nothing wrong.
    run_sysand_in(&cwd, ["sync"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("env is already up to date"));
    Ok(())
}

#[test]
fn verify_reports_unrecorded_directory() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0"])?;
    let stray = cwd.join(DEFAULT_ENV_NAME).join("lib/kpar.stray_1.0.0");
    fs::create_dir_all(&stray)?;
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert().failure().stderr(predicate::str::contains(
        "`lib/kpar.stray_1.0.0` is installed but not recorded in `env.toml`",
    ));
    Ok(())
}

#[test]
fn verify_reports_environment_gone() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0", "dep1"])?;
    fs::remove_dir_all(cwd.join(DEFAULT_ENV_NAME))?;
    let out = run_sysand_in(&cwd, ["sync", "--verify"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "`urn:dep0` 1.0.0 is in `sysand-lock.toml` but not installed; run `sysand sync`",
        ))
        .stderr(predicate::str::contains(
            "`urn:dep1` 1.0.0 is in `sysand-lock.toml` but not installed; run `sysand sync`",
        ))
        .stderr(predicate::str::contains("2 problem(s)"));
    // Verifying did not create the environment.
    assert!(!cwd.join(DEFAULT_ENV_NAME).exists());
    Ok(())
}

#[test]
fn verify_reports_stale_lockfile_and_repair_keeps_it() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0"])?;
    write_app(&cwd, &["dep0"], "^2.0.0")?;
    let lock_before = fs::read(cwd.join(DEFAULT_LOCKFILE_NAME))?;
    let expected = format!(
        "the lockfile does not match the manifests: `urn:dep0` is `^2.0.0` in `{}` and `1.0.0` in `sysand-lock.toml`; run `sysand lock`",
        cwd.join(".project.json")
    );
    run_sysand_in(&cwd, ["sync", "--verify"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(expected.as_str()));
    // Repair does not re-solve: same report, lockfile untouched.
    run_sysand_in(&cwd, ["sync", "--repair"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(expected.as_str()))
        .stderr(predicate::str::contains("1 problem(s) remain"));
    assert_eq!(lock_before, fs::read(cwd.join(DEFAULT_LOCKFILE_NAME))?);
    Ok(())
}

#[test]
fn repair_heals_every_state() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = synced_app(&["dep0", "dep1", "dep2", "dep3"])?;
    // A tampered file, a directory gone, a stray directory and an entry
    // removed from `env.toml` (whose directory then counts as unrecorded
    // too), all at once: five findings.
    fs::write(
        installed(&cwd, "dep0")?.join("dep0.sysml"),
        "package dep0 { part p; }\n",
    )?;
    fs::remove_dir_all(installed(&cwd, "dep1")?)?;
    fs::create_dir_all(cwd.join(DEFAULT_ENV_NAME).join("lib/kpar.stray_1.0.0"))?;
    let env_toml_path = cwd.join(DEFAULT_ENV_NAME).join(METADATA_PATH);
    let env_toml = fs::read_to_string(&env_toml_path)?;
    let dep3_entry = env_toml
        .split("[[project]]")
        .find(|entry| entry.contains("name = \"dep3\""))
        .expect("dep3 entry")
        .to_owned();
    fs::write(
        &env_toml_path,
        env_toml.replacen(&format!("[[project]]{dep3_entry}"), "", 1),
    )?;

    run_sysand_in(&cwd, ["sync", "--verify"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("5 problem(s)"));

    run_sysand_in(&cwd, ["sync", "--repair"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("Repairing"))
        .stderr(predicate::str::contains(
            "repaired 3 project(s); `.sysand` matches `sysand-lock.toml`: 4 project(s) verified",
        ));
    assert!(
        !cwd.join(DEFAULT_ENV_NAME)
            .join("lib/kpar.stray_1.0.0")
            .exists()
    );

    run_sysand_in(&cwd, ["sync", "--verify"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("4 project(s) verified"));

    // The whole environment gone, then repaired.
    fs::remove_dir_all(cwd.join(DEFAULT_ENV_NAME))?;
    run_sysand_in(&cwd, ["sync", "--repair"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("repaired 4 project(s)"));
    Ok(())
}
