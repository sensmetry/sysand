// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use assert_cmd::prelude::*;
use predicates::prelude::*;

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

/// What `sysand init` writes, byte for byte.
///
/// model-toml.md "Manifest version": "`sysand init`, `sysand new` and
/// `sysand migrate` write the current value" -- so a new project is a
/// `model.toml`, in the layout "Editing the manifest" fixes. The two
/// json documents are what a build produces and what a legacy project
/// has.
#[test]
fn sysand_init_writes_a_model_toml() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--version",
            "1.2.3",
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "--license",
            "MIT",
            "project",
        ],
        None,
    )?;
    out.assert().success().stdout(predicate::str::is_empty());

    let project = cwd.join("project");
    assert_eq!(
        std::fs::read_to_string(project.join("model.toml"))?,
        "manifest-version = 1\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         sources = []\n\
         license = \"MIT\"\n"
    );
    assert!(!project.join(".project.json").exists());
    assert!(!project.join(".meta.json").exists());

    Ok(())
}

/// An invalid license is rejected by both `sysand init --license` and
/// `sysand edit --license` with the same message, where the SPDX
/// caret diagram starts on its own line so that it stays aligned.
#[test]
fn invalid_license_error_is_consistent() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "proj",
            "--license",
            "MIT ANDD foo",
        ],
        None,
    )?;
    let expected = "not a valid SPDX license expression:\nMIT ANDD foo\n    ^^^^ unknown term\n";
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));
    assert!(!cwd.join("model.toml").exists());

    let out = run_sysand_in(
        &cwd,
        ["init", "--publisher", "acme", "--name", "proj"],
        None,
    )?;
    out.assert().success();
    let out = run_sysand_in(&cwd, ["edit", "--license", "MIT ANDD foo"], None)?;
    out.assert()
        .failure()
        .stderr(predicate::str::contains(expected));

    Ok(())
}

/// The license goes in verbatim.
///
/// `render_new` writes an SPDX expression through its `Display`, and
/// `spdx::Expression`'s `Display` is the text it parsed -- whitespace
/// included. So this is an echo, and pinning it is what would catch the
/// day it stops being one.
#[test]
fn sysand_init_writes_the_license_expression_as_given() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_ok(&["--license", "Apache-2.0 OR    MIT"])?;

    assert!(
        std::fs::read_to_string(cwd.join("project").join("model.toml"))?
            .contains("license = \"Apache-2.0 OR    MIT\"\n")
    );
    Ok(())
}

/// Without a version, `0.0.1`.
#[test]
fn sysand_init_defaults_the_version() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "project",
        ],
        None,
    )?;
    out.assert().success();

    assert!(
        std::fs::read_to_string(cwd.join("project").join("model.toml"))?
            .contains("version = \"0.0.1\"\n")
    );
    Ok(())
}

/// A name the manifest rule refuses, taken from the directory. The
/// message has to say where the fix is: not at the name, which the user
/// never typed.
#[test]
fn a_name_from_the_directory_names_the_name_flag() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(["init", "--publisher", "acme", "my_project"], None)?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "name must be 3-50 ASCII alphanumeric chars",
        ))
        .stderr(predicate::str::contains(
            "the name came from the directory; pass `--name` to choose another",
        ));
    Ok(())
}

/// The same name, typed. There is nothing left to suggest.
#[test]
fn a_name_from_the_name_flag_is_refused_without_the_hint() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, _cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "my_project",
            "project",
        ],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "name must be 3-50 ASCII alphanumeric chars",
        ))
        .stderr(predicate::str::contains("the name came from the directory").not());
    Ok(())
}

/// A publisher never comes from a directory, so it never gets the hint.
#[test]
fn a_publisher_the_rule_refuses_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, _cwd, out) = run_sysand(
        ["init", "--publisher", "ab", "--name", "widgets", "project"],
        None,
    )?;

    out.assert()
        .failure()
        .stderr(predicate::str::contains(
            "publisher must be 3-50 ASCII alphanumeric chars",
        ))
        .stderr(predicate::str::contains("--name").not());
    Ok(())
}

/// `sysand new` is `sysand init`, as an alias: model-toml.md names both.
#[test]
fn sysand_new_is_sysand_init() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        ["new", "--publisher", "acme", "--name", "widgets", "project"],
        None,
    )?;
    out.assert().success();

    assert!(cwd.join("project").join("model.toml").exists());
    Ok(())
}

/// A generated project is one Sysand reads back.
#[test]
fn a_generated_project_is_read_back_by_info() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_ok(&[])?;

    run_sysand_in(&cwd.join("project"), ["info"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("widgets"))
        .stdout(predicate::str::contains("acme"));
    Ok(())
}

/// `created` is the Unix epoch for a generated project, and that is
/// deliberate: it is computed at build, and a view of a source project
/// must not be a function of the clock or two reads of one project
/// would not agree on its checksum.
///
/// The value appears only through `info`, which is what this test
/// reads.
#[test]
fn created_is_the_epoch_for_a_generated_project() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_ok(&[])?;

    run_sysand_in(&cwd.join("project"), ["info", "--get", "created"], None)?
        .assert()
        .success()
        .stdout(predicate::str::contains("1970-01-01T00:00:00Z"));
    Ok(())
}

/// A second `init` in the same directory.
#[test]
fn initing_twice_says_the_manifest_is_there() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = init_ok(&[])?;

    run_sysand_in(&cwd.join("project"), ["init", "--publisher", "acme"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains("`model.toml` already exists"));
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
            run_sysand(["init", "--publisher", "acme", "--name", name, "p"], None)?;

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
    let manifest = std::fs::read_to_string(cwd.join("p").join("model.toml"))?;
    assert_eq!(
        manifest,
        "manifest-version = 1\n\n\
         [project]\n\
         publisher = \"Acme Labs\"\n\
         name = \"My Project\"\n\
         version = \"0.0.1\"\n\
         sources = []\n"
    );

    Ok(())
}

/// `sysand init` without `--name` should reject a directory name that is
/// not a valid project name, and suggest `--name`. `:` is not allowed in
/// Windows file names, hence Unix only
#[cfg(unix)]
#[test]
fn init_rejects_invalid_directory_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(["init", "--publisher", "acme", "bad:dir"], None)?;

    out.assert().failure().stderr(predicate::str::contains(
        "cannot use the directory name `bad:dir` as the project name: \
         name cannot contain `:`; use `--name` to set it",
    ));
    assert!(!cwd.join("bad:dir").join("model.toml").exists());

    let out = run_sysand_in(
        &cwd,
        ["init", "--publisher", "acme", "--name", "good", "bad:dir"],
        None,
    )?;
    out.assert().success();
    assert!(cwd.join("bad:dir").join("model.toml").exists());

    Ok(())
}

/// `sysand init --metamodel` should write the metamodel of the current
/// release to `model.toml`, and `--metamodel-release` should choose it
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
            ["init", "--publisher", "acme", "proj"]
                .into_iter()
                .chain(args.iter().copied()),
            None,
        )?;

        out.assert().success();
        let manifest = std::fs::read_to_string(cwd.join("proj/model.toml"))?;
        assert!(
            manifest.contains(&format!("metamodel = \"{metamodel}\"\n")),
            "args: {args:?}, manifest: {manifest}"
        );
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
            "acme",
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

/// `init` in a directory that is already a *legacy* project. A
/// different refusal: adding a `model.toml` beside the json pair makes
/// a project nothing can read.
#[test]
fn initing_over_a_json_project_says_to_migrate() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "widgets", "1.2.3", None)?;

    run_sysand_in(
        &cwd,
        ["init", "--publisher", "acme", "--name", "widgets"],
        None,
    )?
    .assert()
    .failure()
    .stderr(predicate::str::contains("is a `.project.json` project"))
    .stderr(predicate::str::contains("sysand migrate"));
    Ok(())
}

/// `sysand init --publisher acme --name widgets project`, plus `extra`.
fn init_ok(
    extra: &[&str],
) -> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let mut args = vec!["init", "--publisher", "acme", "--name", "widgets"];
    args.extend_from_slice(extra);
    args.push("project");
    let (temp_dir, cwd, out) = run_sysand(args, None)?;
    out.assert().success();
    Ok((temp_dir, cwd))
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
            let (publisher, name) =
                if call == "cli_init_project_basic" || call == "write_legacy_project" {
                    // `write_legacy_project(dir, publisher, name, …)` — the
                    // directory is an expression, and `skip_while` above
                    // drops it only when it is a reference.
                    let args: Vec<&str> = args
                        .iter()
                        .copied()
                        .skip(usize::from(call == "write_legacy_project"))
                        .collect();
                    (arg(args.first()), arg(args.get(1)))
                } else {
                    // With no `--name`, the name comes from the directory.
                    (
                        arg(args.get(1)),
                        arg(args.get(2)).or_else(|| arg(args.first())),
                    )
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
        "cli_init_json_project",
        "write_legacy_project",
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
