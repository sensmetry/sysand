// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Where a command run inside a `model.toml` workspace reads and writes
//! its configuration.
//!
//! workspaces.md "Source overrides": "a project-local one must be at the
//! workspace root: overrides apply workspace-wide ... as a member cannot
//! override sources for itself alone."
//!
//! These use [`run_sysand_discovering`], which leaves configuration
//! discovery on: where `sysand.toml` is *found* is the question.

use assert_cmd::prelude::*;
use camino::Utf8Path;
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{self, ConfigProject, OverrideSource},
    lock::Lockfile,
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

const MEMBER: &str = "\
manifest-version = 1

[project]
publisher = \"acme\"
name = \"alpha\"
version = \"1.0.0\"
sources = []
";

/// A workspace root with one member, `app`, and one project the member
/// can override a usage onto, `dep`.
fn workspace(root_text: &str) -> Result<camino_tempfile::Utf8TempDir, Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(dir.path().join("model.toml"), root_text)?;
    std::fs::create_dir_all(dir.path().join("app"))?;
    std::fs::write(dir.path().join("app/model.toml"), MEMBER)?;
    Ok(dir)
}

const ROOT: &str = "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n";

/// A `sysand.toml` whose one override points `urn:kpar:local-dep` at
/// `src_path`, written at `at`.
fn write_config(at: &Utf8Path, src_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = toml::to_string(&config::Config {
        indexes: vec![],
        projects: vec![ConfigProject {
            identifiers: vec![fluent_uri::Iri::parse(USAGE.to_owned()).unwrap()],
            sources: vec![OverrideSource::LocalSrc {
                src_path: src_path.into(),
            }],
        }],
    })?;
    std::fs::write(at.join(config::local_fs::CONFIG_FILE), cfg)?;
    Ok(())
}

/// A project the override can resolve to.
///
/// Written directly rather than through `sysand init`, which inside a
/// workspace appends it to `members`, which would rewrite the fixture
/// under the test.
fn dependency(dir: &Utf8Path, at: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = dir.join(at);
    std::fs::create_dir_all(&path)?;
    write_legacy_project(&path, "acme", "local-dep", "1.0.0", None)?;
    Ok(())
}

/// A usage the member's `model.toml` can express. A bare `urn:` IRI
/// cannot be written into a manifest -- "a manifest usage is named by
/// its publisher and name".
const USAGE: &str = "pkg:sysand/acme/local-dep";

/// A `model.toml` index usage needs a version constraint.
const CONSTRAINT: &str = "1";

fn locked_projects(member: &Utf8Path) -> Result<usize, Box<dyn std::error::Error>> {
    Ok(Lockfile::read(member)?.into_lock().projects.len())
}

/// The override resolves, so the lockfile has the member and the
/// dependency.
#[test]
fn a_config_at_the_workspace_root_is_used() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    dependency(dir.path(), "local-dep")?;
    run_sysand_in(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(dir.path(), "local-dep")?;

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    assert_eq!(locked_projects(&member)?, 2);
    Ok(())
}

/// The same file in the member has no effect: it is not at the root.
#[test]
fn a_config_in_a_member_is_ignored() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    dependency(&member, "local-dep")?;
    run_sysand_in(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(&member, "local-dep")?;

    // No override resolves, so the lock cannot be completed.
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .failure();
    Ok(())
}

/// "overrides apply workspace-wide", so a relative path in the root's
/// file is relative to the root -- not to the member the command is run
/// in.
#[test]
fn a_relative_override_is_relative_to_the_workspace_root() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    // The dependency sits beside the member, so `local-dep` resolves
    // only if the base path is the root.
    dependency(dir.path(), "local-dep")?;
    std::fs::create_dir_all(member.join("local-dep"))?;
    run_sysand_in(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(dir.path(), "local-dep")?;

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    assert_eq!(locked_projects(&member)?, 2);
    Ok(())
}

/// `add --path` records an override; it must land where the next
/// command will read it.
#[test]
fn add_writes_the_config_at_the_workspace_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    dependency(dir.path(), "local-dep")?;

    run_sysand_discovering(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--as-local-src",
            "../local-dep",
            "--no-lock",
        ],
    )?
    .assert()
    .success();

    assert!(
        dir.path().join(config::local_fs::CONFIG_FILE).exists(),
        "the override belongs at the workspace root"
    );
    assert!(
        !member.join(config::local_fs::CONFIG_FILE).exists(),
        "and not in the member"
    );
    Ok(())
}

#[test]
fn outside_a_workspace_the_project_root_is_still_used() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "standalone", "1.2.3", None)?;
    dependency(&cwd, "local-dep")?;
    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(&cwd, "local-dep")?;

    run_sysand_discovering(&cwd, ["lock"])?.assert().success();
    assert_eq!(locked_projects(&cwd)?, 2);
    Ok(())
}

/// A `.workspace.json` tree answers `None` to `discover_table`, so
/// nothing about it changes.
#[test]
fn a_workspace_json_tree_is_unaffected() -> Result<(), Box<dyn std::error::Error>> {
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "legacy", "1.2.3", None)?;
    std::fs::write(
        cwd.join(".workspace.json"),
        br#"{"projects": [{"path": ".", "iris": []}]}"#,
    )?;
    dependency(&cwd, "local-dep")?;
    run_sysand_in(
        &cwd,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(&cwd, "local-dep")?;

    run_sysand_discovering(&cwd, ["lock"])?.assert().success();
    assert_eq!(locked_projects(&cwd)?, 2);
    Ok(())
}

/// Configuration is not about the members.
#[test]
fn a_broken_member_does_not_change_where_config_is_read() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace("manifest-version = 1\n\n[workspace]\nmembers = [\"app\", \"gone\"]\n")?;
    let member = dir.path().join("app");
    dependency(dir.path(), "local-dep")?;
    run_sysand_in(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(dir.path(), "local-dep")?;

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    assert_eq!(locked_projects(&member)?, 2);
    Ok(())
}

// ------------------------------------------------- stray configuration

const ROW_26: &str = "is not at the workspace root, so it is ignored as a whole";

/// The warning names the file, not its directory: a reader told about a
/// directory would have to guess which file in it is meant.
#[test]
fn a_config_in_a_member_warns() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    write_config(&member, "local-dep")?;

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .stderr(predicate::str::contains(format!(
            "`{}` {ROW_26}",
            member.join(config::local_fs::CONFIG_FILE)
        )));
    Ok(())
}

#[test]
fn a_config_between_cwd_and_the_root_warns() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let deep = dir.path().join("app/src");
    std::fs::create_dir_all(&deep)?;
    write_config(&deep, "local-dep")?;

    run_sysand_discovering(&deep, ["lock"])?
        .assert()
        .stderr(predicate::str::contains(ROW_26));
    Ok(())
}

/// Two strays are two mistakes.
#[test]
fn every_stray_config_is_reported() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    let deep = member.join("src");
    std::fs::create_dir_all(&deep)?;
    write_config(&member, "local-dep")?;
    write_config(&deep, "local-dep")?;

    let out = run_sysand_discovering(&deep, ["lock"])?;
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(
        stderr.matches(ROW_26).count(),
        2,
        "both strays must be named: {stderr}"
    );
    Ok(())
}

/// The root's own file is the one being used.
#[test]
fn the_root_config_is_not_reported_as_a_stray() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    dependency(dir.path(), "local-dep")?;
    run_sysand_in(
        &member,
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--no-lock",
        ],
        None,
    )?
    .assert()
    .success();
    write_config(dir.path(), "local-dep")?;

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success()
        .stderr(predicate::str::contains(ROW_26).not());
    Ok(())
}

/// `print-root` reads no configuration, so a warning about one would be
/// noise.
#[test]
fn a_command_that_reads_no_config_does_not_warn() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    write_config(&member, "local-dep")?;

    run_sysand_discovering(&member, ["print-root"])?
        .assert()
        .stderr(predicate::str::contains(ROW_26).not());
    Ok(())
}

/// `info` resolves under `--iri`, and under a `<publisher>/<name>`
/// identifier, which is looked up in the indexes.
#[test]
fn info_warns_with_an_iri_and_with_an_identifier() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    write_config(&member, "local-dep")?;

    run_sysand_discovering(&member, ["info", "--iri", USAGE])?
        .assert()
        .stderr(predicate::str::contains(ROW_26));
    run_sysand_discovering(&member, ["info", "acme/local-dep"])?
        .assert()
        .stderr(predicate::str::contains(ROW_26));
    // Plain `info` reads the project in front of it and resolves
    // nothing.
    run_sysand_discovering(&member, ["info"])?
        .assert()
        .stderr(predicate::str::contains(ROW_26).not());
    Ok(())
}

#[test]
fn no_config_silences_the_warning() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    write_config(&member, "local-dep")?;

    run_sysand_in(&member, ["lock"], None)?
        .assert()
        .stderr(predicate::str::contains(ROW_26).not());
    Ok(())
}

// ------------------------------- which commands consult the workspace

// `workspaces.md` "Command scopes": `clone`, `index` and `auth` are
// scope-free, so a workspace they would refuse does not stop them.

const NESTED_TEXT: &str = "nested workspaces are not supported";
const UNLISTED_TEXT: &str = "is neither listed in `workspace.members`";

/// An outer workspace that does not `exclude` the inner one: appendix A
/// row 24. Returns the inner member directory.
fn nested()
-> Result<(camino_tempfile::Utf8TempDir, camino::Utf8PathBuf), Box<dyn std::error::Error>> {
    let outer = camino_tempfile::tempdir()?;
    std::fs::write(outer.path().join("model.toml"), ROOT)?;
    let inner = outer.path().join("inner");
    std::fs::create_dir_all(&inner)?;
    std::fs::write(inner.join("model.toml"), ROOT)?;
    std::fs::create_dir_all(inner.join("app"))?;
    std::fs::write(inner.join("app/model.toml"), MEMBER)?;
    let member = inner.join("app");
    Ok((outer, member))
}

/// `index init` is the scope-free probe: the only one of the three that
/// is offline and needs neither a project nor a keyring. `--index-root`
/// is required -- without it the index lands in the current directory,
/// inside the fixture.
fn index_init(cwd: &Utf8Path) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    run_sysand_discovering(cwd, ["index", "init", "--index-root", "idx"])
}

#[test]
fn index_init_runs_in_a_nested_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let (_outer, member) = nested()?;
    index_init(&member)?.assert().success();
    Ok(())
}

/// A nested workspace is an error for a command that has a scope.
#[test]
fn a_nested_workspace_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let (_outer, member) = nested()?;
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(NESTED_TEXT));
    Ok(())
}

/// The check runs before configuration is loaded: a command about to
/// refuse the workspace should not first warn about where it would have
/// read `sysand.toml`.
///
/// The stray file sits one directory *below* the member, because in a
/// nested workspace `config_root` falls back to the project root -- the
/// member itself -- so a `sysand.toml` in the member is "the root's own
/// file" and is never reported whatever the ordering.
#[test]
fn a_nested_workspace_is_refused_before_the_config_warning()
-> Result<(), Box<dyn std::error::Error>> {
    let (_outer, member) = nested()?;
    let deep = member.join("src");
    std::fs::create_dir_all(&deep)?;
    write_config(&deep, "local-dep")?;

    run_sysand_discovering(&deep, ["lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(NESTED_TEXT))
        .stderr(predicate::str::contains(ROW_26).not());
    Ok(())
}

/// workspaces.md "Included projects": a project under the root in
/// neither list is a hard error.
#[test]
fn an_unlisted_project_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    run_sysand_discovering(&dir.path().join("stray"), ["lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(UNLISTED_TEXT));
    Ok(())
}

#[test]
fn index_init_runs_in_an_unlisted_project() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    index_init(&dir.path().join("stray"))?.assert().success();
    Ok(())
}

/// `clone` is scope-free *and* reads configuration, so it can still
/// show where `sysand.toml` is read from in a workspace no scoped
/// command would accept.
#[test]
fn clone_runs_in_an_unlisted_project() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    dependency(dir.path(), "src-proj")?;
    run_sysand_discovering(
        &dir.path().join("stray"),
        ["clone", "--dir", "../src-proj", "--target", "out"],
    )?
    .assert()
    .success();
    Ok(())
}

/// "This is the only kind of non-member project that may sit under a
/// workspace root."
#[test]
fn an_excluded_project_is_not_unlisted() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    )?;
    dependency(dir.path(), "vendor/other")?;
    run_sysand_discovering(&dir.path().join("vendor/other"), ["lock"])?
        .assert()
        .success();
    Ok(())
}

/// The reason the scope check reads the table and not the members:
/// `lock` writes a lockfile into the member it locks, and
/// `workspaces.md` "Stray per-member state" makes that an error for a
/// member. A command that loaded the members would refuse the workspace
/// its own previous run created.
#[test]
fn a_members_own_lockfile_does_not_stop_a_later_command() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    assert!(member.join(DEFAULT_LOCKFILE_NAME).exists());
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    Ok(())
}
