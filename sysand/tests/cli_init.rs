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
            "--metamodel",
            SYSML,
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
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"widgets\"\n\
         version = \"1.2.3\"\n\
         sources = []\n\
         license = \"MIT\"\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
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
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "proj",
            "--metamodel",
            "sysml",
        ],
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
            "--metamodel",
            SYSML,
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
    let (_temp_dir, _cwd, out) = run_sysand(
        [
            "init",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "my_project",
        ],
        None,
    )?;

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
            "--metamodel",
            SYSML,
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
        [
            "init",
            "--metamodel",
            SYSML,
            "--publisher",
            "ab",
            "--name",
            "widgets",
            "project",
        ],
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
        [
            "new",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "project",
        ],
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

    run_sysand_in(
        &cwd.join("project"),
        ["init", "--metamodel", SYSML, "--publisher", "acme"],
        None,
    )?
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
            "--metamodel",
            "sysml",
            "p",
        ],
        None,
    )?;

    out.assert().success();
    let manifest = std::fs::read_to_string(cwd.join("p").join("model.toml"))?;
    assert_eq!(
        manifest,
        "manifest-version = \"1.0.0\"\n\n\
         [project]\n\
         publisher = \"Acme Labs\"\n\
         name = \"My Project\"\n\
         version = \"0.0.1\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
    );

    Ok(())
}

/// `sysand init` without `--name` should reject a directory name that is
/// not a valid project name, and suggest `--name`. `:` is not allowed in
/// Windows file names, hence Unix only
#[cfg(unix)]
#[test]
fn init_rejects_invalid_directory_name() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--publisher",
            "acme",
            "--metamodel",
            "sysml",
            "bad:dir",
        ],
        None,
    )?;

    out.assert().failure().stderr(predicate::str::contains(
        "cannot use the directory name `bad:dir` as the project name: \
         name cannot contain `:`; use `--name` to set it",
    ));
    assert!(!cwd.join("bad:dir").join("model.toml").exists());

    let out = run_sysand_in(
        &cwd,
        [
            "init",
            "--publisher",
            "acme",
            "--name",
            "good",
            "--metamodel",
            "sysml",
            "bad:dir",
        ],
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
        .stderr(predicate::str::contains("--metamodel <KIND|URL>"));
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
        [
            "init",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
        ],
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
    let mut args = vec![
        "init",
        "--metamodel",
        SYSML,
        "--publisher",
        "acme",
        "--name",
        "widgets",
    ];
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

// ------------------------------------------------- `init` in a workspace

use camino::{Utf8Path, Utf8PathBuf};

const WORKSPACE: &str = "\
manifest-version = \"1.0.0\"

[workspace]
members = [\"app\"]
";

/// A workspace root with one member, in a temporary directory.
fn workspace(root_text: &str) -> Result<camino_tempfile::Utf8TempDir, Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(dir.path().join("model.toml"), root_text)?;
    Ok(dir)
}

fn member(dir: &Utf8Path, at: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir.join(at))?;
    std::fs::write(
        dir.join(at).join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
             version = \"1.0.0\"\nsources = []\n"
        ),
    )?;
    Ok(())
}

fn members_of(root: &Utf8Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::fs::read_to_string(root.join("model.toml"))?)
}

fn init_in(
    cwd: &Utf8Path,
    target: &str,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    run_sysand_in(
        cwd,
        [
            "init",
            "--metamodel",
            SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            target,
        ],
        None,
    )
}

#[test]
fn init_in_a_workspace_appends_the_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "lib")?.assert().success();

    let text = members_of(dir.path())?;
    assert!(text.contains("\"lib\""), "{text}");
    assert!(dir.path().join("lib").join("model.toml").exists());
    Ok(())
}

/// An absolute entry would be refused by `normalize_relative`, so this
/// fails loudly rather than silently writing the wrong thing.
#[test]
fn init_appends_the_path_relative_to_the_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "nested/lib")?.assert().success();

    let text = members_of(dir.path())?;
    assert!(text.contains("\"nested/lib\""), "{text}");
    Ok(())
}

#[test]
fn init_from_a_subdirectory_appends_relative_to_the_root() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;
    let below: Utf8PathBuf = dir.path().join("nested");
    std::fs::create_dir_all(&below)?;

    init_in(&below, "lib")?.assert().success();

    let text = members_of(dir.path())?;
    assert!(text.contains("\"nested/lib\""), "{text}");
    Ok(())
}

/// The target holds nothing, so an unrefused mutation would *succeed*.
#[test]
fn init_inside_a_member_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "app/sub")?
        .assert()
        .failure()
        .stderr(predicate::str::contains("lies inside member `app`"))
        .stderr(predicate::str::contains("nested members are not supported"));
    Ok(())
}

/// "lies inside member `app`" is false of `app` itself. The directory
/// holds a `model.toml`, so the text is what distinguishes this from
/// the collision `init` would report anyway.
#[test]
fn init_at_a_member_directory_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "app")?
        .assert()
        .failure()
        .stderr(predicate::str::contains("is already member `app`"));
    Ok(())
}

#[test]
fn init_under_an_excluded_directory_is_created_but_not_listed()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    )?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "vendor/thing")?.assert().success();

    assert!(dir.path().join("vendor/thing/model.toml").exists());
    let text = members_of(dir.path())?;
    // The root text names `vendor` in `exclude`; what must not change
    // is `members`.
    assert!(
        text.contains("members = [\"app\"]"),
        "`members` must be left alone: {text}"
    );
    Ok(())
}

#[test]
fn init_at_the_workspace_root_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), ".")?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the workspace root must not contain a project",
        ));
    Ok(())
}

/// workspaces.md "Included projects": a project under the root in
/// neither list is a hard error. Without it the author is told
/// `model.toml` already exists, which says nothing about the workspace.
#[test]
fn init_in_an_unlisted_existing_project_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;
    member(dir.path(), "stray", "delta")?;

    init_in(dir.path(), "stray")?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is neither listed in `workspace.members` nor under",
        ));
    Ok(())
}

/// Creating a new project cannot be blocked by an unrelated member's
/// mistake.
#[test]
fn init_in_a_workspace_with_a_broken_member_still_works() -> Result<(), Box<dyn std::error::Error>>
{
    let dir =
        workspace("manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\", \"gone\"]\n")?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "lib")?.assert().success();

    let text = members_of(dir.path())?;
    assert!(text.contains("\"lib\""), "{text}");
    Ok(())
}

/// The target must be one that reaches `do_init` and *then* fails --
/// an empty directory with a publisher the manifest refuses. A target
/// that already holds a project would be refused as unlisted first, and
/// the test would pass without the ordering being right.
#[test]
fn init_does_not_append_when_the_project_could_not_be_created()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(WORKSPACE)?;
    member(dir.path(), "app", "alpha")?;

    run_sysand_in(
        dir.path(),
        [
            "init",
            "--metamodel",
            SYSML,
            "--publisher",
            "x",
            "--name",
            "widgets",
            "half",
        ],
        None,
    )?
    .assert()
    .failure();

    let text = members_of(dir.path())?;
    assert!(!text.contains("half"), "nothing may be appended: {text}");
    Ok(())
}

/// The rule is keyed on the target, not on cwd.
#[test]
fn init_targeting_outside_the_workspace_from_a_member_succeeds()
-> Result<(), Box<dyn std::error::Error>> {
    let outer = camino_tempfile::tempdir()?;
    let root = outer.path().join("ws");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("model.toml"), WORKSPACE)?;
    member(&root, "app", "alpha")?;

    init_in(&root.join("app"), "../../elsewhere")?
        .assert()
        .success();

    assert!(outer.path().join("elsewhere/model.toml").exists());
    let text = members_of(&root)?;
    assert!(!text.contains("elsewhere"), "{text}");
    Ok(())
}

/// `init` edits through the editor rather than re-rendering the file.
#[test]
fn the_appended_entry_leaves_the_rest_of_the_file_alone() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(
        "# the workspace\nmanifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\n  \"app\", # first\n]\n",
    )?;
    member(dir.path(), "app", "alpha")?;

    init_in(dir.path(), "lib")?.assert().success();

    let text = members_of(dir.path())?;
    assert!(text.starts_with("# the workspace\n"), "{text}");
    assert!(text.contains("# first"), "{text}");
    assert!(text.contains("\"lib\""), "{text}");
    Ok(())
}

// ------------------------------------------------------ the metamodel

/// Like [`init_in`], but without the argument: what the workspace says
/// is the whole of the answer.
fn init_in_without_metamodel(
    cwd: &Utf8Path,
    target: &str,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    run_sysand_in(
        cwd,
        ["init", "--publisher", "acme", "--name", "widgets", target],
        None,
    )
}

fn manifest_of(dir: &Utf8Path, at: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::fs::read_to_string(dir.join(at).join("model.toml"))?)
}

/// model-toml.md "Editing the manifest": `init`
/// "take[s] it as a mandatory argument, erroring before writing
/// anything if it is absent, with the message listing the four values".
#[test]
fn init_without_a_metamodel_is_refused() -> Result<(), Box<dyn std::error::Error>> {
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
    out.assert()
        .failure()
        .stderr(predicate::str::contains("`--metamodel <url>` is required"))
        .stderr(predicate::str::contains(
            "`https://www.sysand.com/spec/SysML/<date>`",
        ));
    assert!(
        !cwd.join("project").join("model.toml").exists(),
        "nothing was written"
    );
    Ok(())
}

#[test]
fn init_writes_the_metamodel_it_is_given() -> Result<(), Box<dyn std::error::Error>> {
    let kerml = "https://www.omg.org/spec/KerML/20250201";
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--metamodel",
            kerml,
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
        manifest_of(&cwd, "project")?.contains(&format!("\nmetamodel = \"{kerml}\"\n")),
        "{}",
        manifest_of(&cwd, "project")?
    );
    Ok(())
}

#[test]
fn init_refuses_an_unrecognized_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd, out) = run_sysand(
        [
            "init",
            "--metamodel",
            "https://example.com/spec",
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "project",
        ],
        None,
    )?;
    out.assert().failure().stderr(predicate::str::contains(
        "`--metamodel` is not a metamodel Sysand recognizes: `https://example.com/spec`",
    ));
    assert!(
        !cwd.join("project").join("model.toml").exists(),
        "nothing was written"
    );
    Ok(())
}

/// The one case where the argument may be omitted: "inside a workspace
/// whose default group defines `metamodel` [...] the generated file
/// carries `metamodel.workspace = true`".
#[test]
fn init_in_a_workspace_inherits_the_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;

    init_in_without_metamodel(dir.path(), "lib")?
        .assert()
        .success();

    let text = manifest_of(dir.path(), "lib")?;
    assert!(text.contains("\nmetamodel.workspace = true\n"), "{text}");
    Ok(())
}

/// The same, inside a workspace: a default group that declares no
/// `metamodel` has nothing to inherit.
#[test]
fn init_in_a_workspace_without_a_group_metamodel_is_refused()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace("manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = []\n")?;

    init_in_without_metamodel(dir.path(), "lib")?
        .assert()
        .failure()
        .stderr(predicate::str::contains("`--metamodel <url>` is required"));
    assert!(!dir.path().join("lib").join("model.toml").exists());
    Ok(())
}

/// "A named group is not guessed at either -- to inherit from one, the
/// member's marker is written afterwards."
#[test]
fn init_does_not_guess_a_named_group() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = []\n\n\
         [workspace.group.kerml]\n\
         metamodel = \"https://www.omg.org/spec/KerML/20250201\"\n",
    )?;

    init_in_without_metamodel(dir.path(), "lib")?
        .assert()
        .failure()
        .stderr(predicate::str::contains("`--metamodel <url>` is required"));
    Ok(())
}

/// An argument that is given wins: a group is what a project inherits
/// when it says nothing, not something that overrides what it was told.
#[test]
fn init_prefers_the_argument_to_the_group() -> Result<(), Box<dyn std::error::Error>> {
    let kerml = "https://www.omg.org/spec/KerML/20250201";
    let dir = workspace(
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    )?;

    run_sysand_in(
        dir.path(),
        [
            "init",
            "--metamodel",
            kerml,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "lib",
        ],
        None,
    )?
    .assert()
    .success();

    let text = manifest_of(dir.path(), "lib")?;
    assert!(
        text.contains(&format!("\nmetamodel = \"{kerml}\"\n")),
        "{text}"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// `init` with a Syside metamodel (design/syside-metamodel.md,
// "Commands").

const SYSIDE_SYSML: &str = "https://www.sysand.com/spec/SysML/20250201";
const SYSIDE_KERML: &str = "https://www.sysand.com/spec/KerML/20250201";

/// The Syside layout, byte for byte: `sources` listing one file, `root`
/// naming it, and the file itself with a package declaration. The
/// project then reads without a warning and builds -- the whole strand
/// from `init`.
#[test]
fn init_with_a_syside_metamodel_writes_the_syside_layout() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--metamodel",
            SYSIDE_SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "--version",
            "1.2.3",
            "project",
        ],
        None,
    )?
    .assert()
    .success();

    let project = cwd.join("project");
    assert_eq!(
        std::fs::read_to_string(project.join("model.toml"))?,
        format!(
            "manifest-version = \"1.0.0\"\n\n\
             [project]\n\
             publisher = \"acme\"\n\
             name = \"widgets\"\n\
             version = \"1.2.3\"\n\
             sources = [\n    \"widgets.sysml\",\n]\n\
             metamodel = \"{SYSIDE_SYSML}\"\n\
             root = \"widgets.sysml\"\n"
        )
    );
    assert_eq!(
        std::fs::read_to_string(project.join("widgets.sysml"))?,
        "package widgets;\n"
    );

    run_sysand_in(&project, ["info"], None)?
        .assert()
        .success()
        .stderr(predicate::str::contains("warning").not());
    run_sysand_in(&project, ["build", "./widgets.kpar"], None)?
        .assert()
        .success();
    Ok(())
}

#[test]
fn init_with_a_syside_kerml_metamodel_writes_a_kerml_root() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--metamodel",
            SYSIDE_KERML,
            "--publisher",
            "acme",
            "--name",
            "kernel",
            "project",
        ],
        None,
    )?
    .assert()
    .success();
    let manifest = std::fs::read_to_string(cwd.join("project/model.toml"))?;
    assert!(manifest.contains("root = \"kernel.kerml\""), "{manifest}");
    assert!(cwd.join("project/kernel.kerml").is_file());
    Ok(())
}

/// An existing root file is left alone.
#[test]
fn init_does_not_overwrite_an_existing_root_file() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::create_dir_all(cwd.join("project"))?;
    std::fs::write(cwd.join("project/widgets.sysml"), "package Widgets { }\n")?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--metamodel",
            SYSIDE_SYSML,
            "--publisher",
            "acme",
            "--name",
            "widgets",
            "project",
        ],
        None,
    )?
    .assert()
    .success();
    assert_eq!(
        std::fs::read_to_string(cwd.join("project/widgets.sysml"))?,
        "package Widgets { }\n"
    );
    Ok(())
}

/// Inside a workspace whose default group declares a Syside metamodel,
/// the member inherits it by marker and still gets the Syside layout.
#[test]
fn init_in_a_workspace_with_a_syside_default_group_writes_the_layout_and_the_marker()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    std::fs::write(
        cwd.join("model.toml"),
        format!(
            "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = []\nmetamodel = \"{SYSIDE_SYSML}\"\n"
        ),
    )?;
    run_sysand_in(
        &cwd,
        ["init", "--publisher", "acme", "--name", "widgets", "app"],
        None,
    )?
    .assert()
    .success();
    let manifest = std::fs::read_to_string(cwd.join("app/model.toml"))?;
    assert!(
        manifest.contains("metamodel.workspace = true"),
        "{manifest}"
    );
    assert!(manifest.contains("root = \"widgets.sysml\""), "{manifest}");
    assert!(cwd.join("app/widgets.sysml").is_file());
    run_sysand_in(&cwd, ["lock", "--no-index"], None)?
        .assert()
        .success();
    Ok(())
}

/// A name that is not a KerML/SysML identifier is a quoted package name.
#[test]
fn init_with_a_name_that_is_not_an_identifier_quotes_the_package()
-> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = new_temp_cwd()?;
    run_sysand_in(
        &cwd,
        [
            "init",
            "--metamodel",
            SYSIDE_SYSML,
            "--publisher",
            "acme",
            "--name",
            "3D Models",
            "project",
        ],
        None,
    )?
    .assert()
    .success();
    assert_eq!(
        std::fs::read_to_string(cwd.join("project/3d-models.sysml"))?,
        "package '3d-models';\n"
    );
    run_sysand_in(&cwd.join("project"), ["build", "./out.kpar"], None)?
        .assert()
        .success();
    Ok(())
}
