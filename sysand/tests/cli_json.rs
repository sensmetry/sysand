// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `--format json`: one document on stdout per run, carrying the
//! envelope; warnings on stderr; mutating commands report a result,
//! failures a failure document; two runs on an unchanged workspace agree byte for byte.
//!
//! Golden documents live in `tests/data/json/`; `<ROOT>` in them stands
//! for the temporary workspace root.

use std::{error::Error, fs};

use assert_cmd::prelude::*;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use predicates::prelude::*;
use serde_json::Value;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    lock::{Lock, Lockfile, Project, Source, Usage},
    project::{ProjectRead as _, local_src::LocalSrcProject},
    utils::sha256_lowercase_hex,
    workspace_lock::{LockMode, WorkspaceLock},
};
use typed_path::Utf8UnixPathBuf;

mod common;
pub use common::*;

const DEP_IRI: &str = "urn:dep";

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

/// A json app using `dep` (a local source under `deps/dep`), with a
/// lockfile, not yet synced.
fn locked_app() -> Result<(Utf8TempDir, Utf8PathBuf), Box<dyn Error>> {
    let (temp_dir, cwd) = new_temp_cwd()?;
    fs::write(
        cwd.join(".project.json"),
        r#"{"name":"app","publisher":"acme","version":"0.1.0","description":"an app","usage":[{"resource":"urn:dep","versionConstraint":"^1.0.0"}]}"#,
    )?;
    fs::write(cwd.join("app.sysml"), "package app { }\n")?;
    fs::write(
        cwd.join(".meta.json"),
        r#"{"index":{"app":"app.sysml"},"created":"2026-01-01T00:00:00Z"}"#,
    )?;
    let dir = cwd.join("deps/dep");
    write_dep(&dir, "dep")?;
    let checksum = LocalSrcProject::new_access(&dir, None)
        .checksum_canonical_hex()?
        .expect("manifests");
    let lock = Lock {
        projects: vec![
            Project {
                publisher: Some("acme".to_owned()),
                name: "app".to_owned(),
                version: "0.1.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec![],
                usages: vec![Usage::from_str_unchecked(DEP_IRI)],
                sources: vec![Source::Editable {
                    editable: Utf8UnixPathBuf::from("."),
                }],
            },
            Project {
                publisher: Some("acme".to_owned()),
                name: "dep".to_owned(),
                version: "1.0.0".to_owned(),
                metamodel: None,
                exports: vec![],
                identifiers: vec![DEP_IRI.to_owned()],
                usages: vec![],
                sources: vec![Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("deps/dep"),
                    checksum,
                }],
            },
        ],
        ..Lock::default()
    };
    Lockfile::new(&cwd, lock).write()?;
    Ok((temp_dir, cwd))
}

/// Run and parse the one document on stdout.
fn json<'a, I: IntoIterator<Item = &'a str>>(
    cwd: &Utf8Path,
    args: I,
) -> Result<(Value, std::process::Output), Box<dyn Error>> {
    let args: Vec<&str> = args.into_iter().chain(["--format", "json"]).collect();
    let out = run_sysand_in(cwd, args, None)?;
    let stdout = String::from_utf8(out.stdout.clone())?;
    let value: Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("stdout is not one JSON document: {e}\n{stdout}"))?;
    Ok((value, out))
}

fn assert_envelope(document: &Value, complete: bool) {
    assert_eq!(document["schemaVersion"], 1, "{document}");
    assert_eq!(document["complete"], complete, "{document}");
    assert_eq!(document["skipped"], Value::Array(vec![]), "{document}");
    assert!(document["escapeHatch"].is_null(), "{document}");
}

/// Compare with a golden document, `<ROOT>` standing for `root`.
fn assert_golden(document: &Value, name: &str, root: &Utf8Path) -> Result<(), Box<dyn Error>> {
    let path =
        Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/data/json/{name}.json"));
    let text = fs::read_to_string(&path)?.replace("<ROOT>", root.as_str());
    let expected: Value = serde_json::from_str(&text)?;
    assert_eq!(
        document,
        &expected,
        "{name}: got\n{}",
        serde_json::to_string_pretty(document)?
    );
    Ok(())
}

#[test]
fn every_document_carries_the_envelope() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    for args in [
        vec!["info"],
        vec!["info", "--get", "name"],
        vec!["sources"],
        vec!["env", "list"],
        vec!["print-root"],
        vec!["sync"],
        vec!["sync", "--verify"],
    ] {
        let (document, out) = json(&cwd, args.clone())?;
        out.assert().success();
        assert_envelope(&document, true);
    }
    Ok(())
}

#[test]
fn info_documents() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let (document, _) = json(&cwd, ["info"])?;
    assert_golden(&document, "info", &cwd)?;
    // The same project through `--path`.
    let (by_path, _) = json(&cwd, ["info", "--dir", "deps/dep"])?;
    assert_eq!(by_path["project"]["name"], "dep");
    assert_eq!(by_path["metadata"]["index"]["dep"], "dep.sysml");
    // A getter, and an absent optional as an empty list.
    let (name, _) = json(&cwd, ["info", "--get", "name"])?;
    assert_eq!(
        name["values"],
        serde_json::json!([{ "member": null, "values": ["app"] }])
    );
    let (license, _) = json(&cwd, ["info", "--get", "license"])?;
    assert_eq!(
        license["values"],
        serde_json::json!([{ "member": null, "values": [] }])
    );
    Ok(())
}

#[test]
fn sources_env_list_and_root_documents() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    let (sources, _) = json(&cwd, ["sources"])?;
    let listed: Vec<&str> = sources["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        listed.iter().any(|p| p.ends_with("app.sysml")),
        "{listed:?}"
    );
    assert!(
        listed.iter().any(|p| p.ends_with("dep.sysml")),
        "{listed:?}"
    );
    let mut sorted = listed.clone();
    sorted.sort_unstable();
    assert_eq!(listed, sorted, "a defined order");

    let (env, _) = json(&cwd, ["env", "list"])?;
    assert_golden(&env, "env-list", &cwd)?;

    let (root, _) = json(&cwd, ["print-root"])?;
    assert_eq!(root["root"], cwd.as_str());
    Ok(())
}

#[test]
fn lock_document() -> Result<(), Box<dyn Error>> {
    // A project `lock --no-index` can solve: a directory usage.
    let (_temp_dir, cwd) = new_temp_cwd()?;
    write_dep(&cwd.join("deps/dep"), "dep")?;
    fs::write(
        cwd.join(".project.json"),
        r#"{"name":"app","publisher":"acme","version":"0.1.0","usage":[{"dir":"deps/dep","publisher":"acme","name":"dep"}]}"#,
    )?;
    fs::write(
        cwd.join(".meta.json"),
        r#"{"index":{},"created":"2026-01-01T00:00:00Z"}"#,
    )?;
    let (lock, out) = json(&cwd, ["lock", "--no-index"])?;
    out.assert().success();
    assert_eq!(lock["lockfile"], cwd.join(DEFAULT_LOCKFILE_NAME).as_str());
    let projects = lock["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 2);
    assert!(
        projects
            .iter()
            .any(|p| p["name"] == "dep" && p["sources"][0].as_str().unwrap().contains("src_path"))
    );
    assert!(
        projects
            .iter()
            .any(|p| p["name"] == "app" && p["sources"][0].as_str().unwrap().contains("editable"))
    );
    Ok(())
}

#[test]
fn sync_documents() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let (first, _) = json(&cwd, ["sync"])?;
    assert_golden(&first, "sync-first", &cwd)?;
    let (second, _) = json(&cwd, ["sync"])?;
    assert_golden(&second, "sync-second", &cwd)?;

    let (verify, out) = json(&cwd, ["sync", "--verify"])?;
    out.assert().success();
    assert_eq!(
        verify["verify"],
        serde_json::json!({ "clean": true, "problems": 0, "verified": 1 })
    );
    assert_eq!(
        verify["verified"],
        serde_json::json!([{ "iri": DEP_IRI, "version": "1.0.0" }])
    );

    // A finding: the document carries it and the run fails with kind
    // `verify`.
    fs::remove_dir_all(cwd.join(".sysand/lib").read_dir()?.next().unwrap()?.path())?;
    let (broken, out) = json(&cwd, ["sync", "--verify"])?;
    out.assert().failure();
    assert_envelope(&broken, false);
    assert_eq!(broken["error"]["kind"], "verify");
    assert_eq!(broken["verify"]["problems"], 1);
    assert_eq!(broken["mismatched"][0]["kind"], "missing");
    Ok(())
}

#[test]
fn versions_verb_text_and_json() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let folder = cwd.join("mirror");
    fs::create_dir_all(&folder)?;
    for version in ["0.1.0", "0.2.0"] {
        let (bytes, _, _) = build_index_kpar_bytes("mock", "dep", version);
        fs::write(folder.join(format!("dep-{version}.kpar")), bytes)?;
    }
    let out = run_sysand_in(
        &cwd,
        [
            "versions",
            "pkg:sysand/mock/dep",
            "--default-index",
            folder.as_str(),
        ],
        None,
    )?;
    out.assert().success().stdout("0.2.0\n0.1.0\n");
    let (document, _) = json(
        &cwd,
        [
            "versions",
            "pkg:sysand/mock/dep",
            "--default-index",
            folder.as_str(),
        ],
    )?;
    assert_golden(&document, "versions", &cwd)?;
    Ok(())
}

#[test]
fn result_documents() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let (init, _) = json(
        &cwd,
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "fresh",
            "--metamodel",
            "https://www.omg.org/spec/SysML/20250201",
        ],
    )?;
    assert_eq!(init["command"], "init");
    assert_eq!(init["changed"], serde_json::json!([cwd.as_str()]));

    fs::write(cwd.join("more.sysml"), "package more { }\n")?;
    let (include, _) = json(&cwd, ["include", "more.sysml"])?;
    assert_eq!(include["command"], "include");
    assert_eq!(
        include["changed"],
        serde_json::json!([cwd.join("model.toml").as_str()])
    );

    let (set, _) = json(&cwd, ["edit", "--name", "renamed"])?;
    assert_eq!(set["command"], "edit");
    assert_eq!(
        set["changed"],
        serde_json::json!([cwd.join("model.toml").as_str()])
    );

    let (env, _) = json(&cwd, ["env"])?;
    assert_eq!(env["command"], "env");
    assert_eq!(
        env["changed"],
        serde_json::json!([cwd.join(".sysand").as_str()])
    );
    Ok(())
}

#[test]
fn failure_document_on_error() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    let (document, out) = json(&cwd, ["sources"])?;
    assert_eq!(out.status.code(), Some(1));
    assert_envelope(&document, false);
    assert_eq!(document["error"]["kind"], "error");
    assert!(
        document["error"]["message"]
            .as_str()
            .unwrap()
            .contains("project")
    );
    out.assert().stderr(predicate::str::contains("error:"));
    Ok(())
}

#[test]
fn failure_kind_lock_held() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    let _held = WorkspaceLock::try_acquire(&cwd, LockMode::Exclusive)?;
    let (document, out) = json(&cwd, ["sync"])?;
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(document["error"]["kind"], "lock-held");
    assert_eq!(
        document["error"]["message"],
        format!("another process is using `{cwd}`")
    );
    Ok(())
}

#[test]
fn auth_refuses_json() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    for (args, name) in [
        (vec!["auth", "status"], "auth status"),
        (vec!["auth", "whoami", "https://example.org"], "auth whoami"),
    ] {
        let (document, out) = json(&cwd, args)?;
        out.assert()
            .failure()
            .stderr(predicate::str::contains(format!(
                "`--format json` is not supported by `{name}`"
            )));
        assert_eq!(
            document["error"]["message"],
            format!("`--format json` is not supported by `{name}`")
        );
    }
    Ok(())
}

#[test]
fn stdout_holds_only_the_document_when_a_warning_is_emitted() -> Result<(), Box<dyn Error>> {
    // `info exports` on an OMG-shaped project warns on stderr; the
    // document is the getter's empty answer.
    let (_temp_dir, cwd) = locked_app()?;
    let (document, out) = json(&cwd, ["info", "--get", "exports"])?;
    out.assert()
        .success()
        .stderr(predicate::str::contains("warning"));
    assert_eq!(
        document["values"],
        serde_json::json!([{ "member": null, "values": [] }])
    );
    Ok(())
}

#[test]
fn two_runs_are_byte_identical_and_field_order_is_not_load_bearing() -> Result<(), Box<dyn Error>> {
    let (_temp_dir, cwd) = locked_app()?;
    run_sysand_in(&cwd, ["sync"], None)?.assert().success();
    for args in [
        vec!["info"],
        vec!["sources"],
        vec!["env", "list"],
        vec!["sync", "--verify"],
    ] {
        let first = run_sysand_in(&cwd, args.iter().copied().chain(["--format", "json"]), None)?;
        let second = run_sysand_in(&cwd, args.iter().copied().chain(["--format", "json"]), None)?;
        assert_eq!(first.stdout, second.stdout, "{args:?}");
        // Semantic equality does not depend on key order.
        let a: Value = serde_json::from_slice(&first.stdout)?;
        let b: Value = serde_json::from_slice(&second.stdout)?;
        assert_eq!(a, b);
    }
    Ok(())
}
