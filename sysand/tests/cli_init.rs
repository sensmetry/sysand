// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, falling back
/// on directory name as name.
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--publisher",
            "acme",
            "init_basic",
        ],
        None,
    )?;

    let proj_dir_path = cwd.join("init_basic");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "init_basic",
  "publisher": "acme",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init`, when not given a version, should default to `0.0.1`.
#[test]
fn init_default_version() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) =
        cli_init_project(Some("init-default-version"), "acme", None, None, None)?;

    let proj_dir_path = cwd.join("init-default-version");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;

    assert_eq!(
        info,
        r#"{
  "name": "init-default-version",
  "publisher": "acme",
  "version": "0.0.1"
}
"#
    );

    Ok(())
}

/// `sysand init`, when not given a directory, should create a
/// project in cwd.
#[test]
fn init_basic_cwd() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = cli_init_project_basic("bravo", "init-basic-cwd", "1.2.3")?;

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "init-basic-cwd",
  "publisher": "bravo",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the specified directory, using explicitly
/// specified name as project name.
#[test]
fn init_explicit_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--publisher",
            "charlie",
            "--name",
            "other_than_init_explicit_name",
            "init_explicit_name",
        ],
        None,
    )?;

    let proj_dir_path = cwd.join("init_explicit_name");

    out.assert().success().stdout(predicate::str::is_empty());

    let info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    let meta_match = predicate::str::is_match(
        r#"\{\n  "index": \{\},\n  "created": "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z"\n}\n"#,
    )?;

    assert_eq!(
        info,
        r#"{
  "name": "other_than_init_explicit_name",
  "publisher": "charlie",
  "version": "1.2.3"
}
"#
    );
    // Isn't there some nicer way to use this?
    assert!(meta_match.eval(&meta));

    Ok(())
}

/// An invalid license is rejected by both `sysand init --license` and
/// `sysand edit --license` with the same message, where the SPDX
/// caret diagram starts on its own line so that it stays aligned.
#[test]
fn invalid_license_error_is_consistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["init", "--publisher", "a", "--license", "MIT ANDD foo"],
        None,
    )?;
    let expected = "not a valid SPDX license expression:\nMIT ANDD foo\n    ^^^^ unknown term\n";
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));
    assert!(!cwd.join(".project.json").exists());

    let out = run_sysand_in(&cwd, ["init", "--publisher", "a"], None)?;
    out.assert().success();
    let out = run_sysand_in(&cwd, ["edit", "--license", "MIT ANDD foo"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the specified directory). Such an existing
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "acme",
            "--version",
            "1.2.3",
            "init_fail_on_double_init",
        ],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    let proj_dir_path = cwd.join("init_fail_on_double_init");

    assert!(proj_dir_path.exists());

    let original_info = std::fs::read_to_string(proj_dir_path.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(proj_dir_path.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "acme",
            "--version",
            "1.2.3",
            "init_fail_on_double_init",
        ],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("`.project.json` already exists"));

    assert_eq!(
        original_info,
        std::fs::read_to_string(proj_dir_path.join(".project.json"))?
    );
    assert_eq!(
        original_meta,
        std::fs::read_to_string(proj_dir_path.join(".meta.json"))?
    );

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the current working directory). The current
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init_cwd() -> Result<(), Box<dyn std::error::Error>> {
    // Run 1
    let (_temp_dir, cwd, out) =
        cli_init_project_basic("acme", "init-fail-on-double-init-cwd", "1.2.3")?;
    out.assert().success().stdout(predicate::str::is_empty());

    let original_info = std::fs::read_to_string(cwd.join(".project.json"))?;
    let original_meta = std::fs::read_to_string(cwd.join(".meta.json"))?;

    // Run 2
    let out_again = run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "init_fail_on_double_init_cwd_again",
            "--version",
            "3.2.1",
        ],
        None,
    )?;
    out_again
        .assert()
        .failure()
        .stderr(predicate::str::contains("`.project.json` already exists"));

    assert_eq!(
        original_info,
        std::fs::read_to_string(cwd.join(".project.json"))?
    );
    assert_eq!(
        original_meta,
        std::fs::read_to_string(cwd.join(".meta.json"))?
    );

    Ok(())
}

/// `sysand init` should reject an invalid `--publisher` before creating
/// anything
#[test]
fn init_rejects_invalid_publisher() -> Result<(), Box<dyn std::error::Error>> {
    for (publisher, msg) in [
        ("", "publisher cannot be empty"),
        ("acme/labs", "publisher cannot contain `/`"),
        ("acme:labs", "publisher cannot contain `:`"),
        ("acme\tlabs", "publisher cannot contain `\\t`"),
    ] {
        let (_temp_dir, cwd, out) =
            run_sysand(["init", "--publisher", publisher, "--name", "n", "p"], None)?;

        out.assert().failure().stderr(
            predicate::str::contains(format!(
                "invalid value '{publisher}' for '--publisher <PUBLISHER>'"
            ))
            .and(predicate::str::contains(msg)),
        );
        assert!(!cwd.join("p").exists(), "publisher: {publisher:?}");
    }

    Ok(())
}

/// `sysand init` should reject an invalid `--name` before creating
/// anything
#[test]
fn init_rejects_invalid_name() -> Result<(), Box<dyn std::error::Error>> {
    for (name, msg) in [
        ("", "name cannot be empty"),
        ("a/b", "name cannot contain `/`"),
        ("a:b", "name cannot contain `:`"),
        ("a\nb", "name cannot contain `\\n`"),
    ] {
        let (_temp_dir, cwd, out) =
            run_sysand(["init", "--publisher", "a", "--name", name, "p"], None)?;

        out.assert().failure().stderr(
            predicate::str::contains(format!("invalid value '{name}' for '--name <NAME>'"))
                .and(predicate::str::contains(msg)),
        );
        assert!(!cwd.join("p").exists(), "name: {name:?}");
    }

    Ok(())
}

/// `sysand init` should accept a publisher and name containing spaces
#[test]
fn init_accepts_spaces_in_publisher_and_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "Acme Labs",
            "--name",
            "My Project",
            "p",
        ],
        None,
    )?;

    out.assert().success();
    let info = std::fs::read_to_string(cwd.join("p").join(".project.json"))?;
    assert_eq!(
        info,
        r#"{
  "name": "My Project",
  "publisher": "Acme Labs",
  "version": "0.0.1"
}
"#
    );

    Ok(())
}

/// `sysand init` without `--name` should reject a directory name that is
/// not a valid project name, and suggest `--name`. `:` is not allowed in
/// Windows file names, hence Unix only
#[cfg(unix)]
#[test]
fn init_rejects_invalid_directory_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(["init", "--publisher", "a", "bad:dir"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "cannot use the directory name `bad:dir` as the project name: \
         name cannot contain `:`; use `--name` to set it",
    ));
    assert!(!cwd.join("bad:dir").join(".project.json").exists());

    let out = run_sysand_in(
        &cwd,
        ["init", "--publisher", "a", "--name", "good", "bad:dir"],
        None,
    )?;
    out.assert().success();
    assert!(cwd.join("bad:dir").join(".project.json").exists());

    Ok(())
}

/// `sysand init --metamodel` should write the metamodel of the current
/// release to `.meta.json`, and `--metamodel-release` should choose it
#[test]
fn init_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    for (args, metamodel) in [
        (
            &["--metamodel", "sysml"][..],
            "https://www.omg.org/spec/SysML/20250201",
        ),
        (
            &["--metamodel", "kerml", "--metamodel-release", "20250201"][..],
            "https://www.omg.org/spec/KerML/20250201",
        ),
    ] {
        let (_temp_dir, cwd, out) = run_sysand(
            ["init", "--publisher", "a", "p"]
                .into_iter()
                .chain(args.iter().copied()),
            None,
        )?;

        out.assert().success();
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(cwd.join("p/.meta.json"))?)?;
        assert_eq!(meta["metamodel"], metamodel, "args: {args:?}");
    }

    Ok(())
}

/// `sysand init` should reject `--metamodel-release` without `--metamodel`
/// before creating anything
#[test]
fn init_metamodel_release_requires_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "a",
            "--metamodel-release",
            "20250201",
            "p",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains("--metamodel <KIND>"));
    assert!(!cwd.join("p").exists());

    Ok(())
}

/// Every identifier the suite creates a project with has to be one the
/// manifest rule accepts.
///
/// `init` itself refuses such an identifier, but a `.project.json` is
/// never validated on write, so a hand-written legacy fixture can pass
/// while describing a project no manifest could. This checks every call
/// site without running the whole suite.
#[test]
fn every_identifier_the_suite_inits_with_is_valid() -> Result<(), Box<dyn std::error::Error>> {
    use sysand_core::model::{IndexName, IndexPublisher};

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default()
            .to_owned();
        let text = std::fs::read_to_string(&path)?;

        for (call, at) in call_sites(&text) {
            let args = arguments(&text[at..]);
            // `_basic(publisher, name, version)`; the others take a
            // leading `&cwd` and/or a `path`, then publisher and name.
            let args: Vec<&str> = if call == "cli_init_project_basic" {
                args
            } else {
                args.into_iter()
                    .skip_while(|a| a.starts_with('&'))
                    .collect()
            };
            let (publisher, name) = if call == "cli_init_project_basic" {
                (arg(args.first()), arg(args.get(1)))
            } else {
                // With no `--name`, the name comes from the directory.
                (arg(args.get(1)), arg(args.get(2)).or(arg(args.first())))
            };
            let Some(publisher) = publisher else { continue };

            assert!(
                IndexPublisher::parse(publisher).is_ok(),
                "{file}: `{publisher}` is not a publisher a `model.toml` can hold"
            );
            if let Some(name) = name {
                assert!(
                    IndexName::parse(name).is_ok(),
                    "{file}: `{name}` is not a name a `model.toml` can hold"
                );
            }
            checked += 1;
        }
    }

    assert!(checked > 100, "the scan found only {checked} call sites");
    Ok(())
}

/// Where each `cli_init_project*` call's argument list begins.
fn call_sites(text: &str) -> Vec<(&'static str, usize)> {
    let mut found = vec![];
    for call in [
        "cli_init_project_basic",
        "cli_init_project_in",
        "cli_init_project",
    ] {
        let mut from = 0;
        while let Some(at) = text[from..].find(&format!("{call}(")) {
            let open = from + at + call.len() + 1;
            // `cli_init_project` is a prefix of the other two.
            if !found.iter().any(|(_, other)| *other == open) {
                found.push((call, open));
            }
            from = open;
        }
    }
    found
}

/// The comma-separated arguments of one call, trimmed.
fn arguments(text: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut args = vec![];
    let mut start = 0usize;
    let mut in_string = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' if !text[..i].ends_with('\\') => in_string = !in_string,
            _ if in_string => {}
            '(' | '[' => depth += 1,
            ')' if depth == 0 => {
                args.push(text[start..i].trim());
                return args;
            }
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(text[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    args
}

/// The string inside `"x"` or `Some("x")`; `None` for anything else.
fn arg<'a>(raw: Option<&&'a str>) -> Option<&'a str> {
    let raw = raw?.trim();
    let inner = raw
        .strip_prefix("Some(")
        .map_or(raw, |rest| rest.strip_suffix(')').unwrap_or(rest));
    inner.strip_prefix('"')?.strip_suffix('"')
}
