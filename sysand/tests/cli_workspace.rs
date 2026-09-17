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
use camino::{Utf8Path, Utf8PathBuf};
use predicates::prelude::*;
use sysand_core::{
    commands::lock::DEFAULT_LOCKFILE_NAME,
    config::{self, ConfigProject, OverrideSource},
    env::DEFAULT_ENV_NAME,
    lock::Lockfile,
};

// pub due to https://github.com/rust-lang/rust/issues/46379
mod common;
pub use common::*;

const MEMBER: &str = "\
manifest-version = \"1.0.0\"

[project]
publisher = \"acme\"
name = \"alpha\"
version = \"1.0.0\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
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

/// `local-dep` and `src-proj` are excluded because
/// `dependency` writes them *under the root*: a project there that is
/// in neither list is refused, and a fixture that relies on
/// one being read is relying on the rule not being enforced. `stray` is
/// deliberately *not* excluded: the tests that use it are about what an
/// unlisted project does.
const ROOT: &str = "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
     exclude = [\"local-dep\", \"src-proj\"]\n";

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
    assert_eq!(locked_projects(dir.path())?, 2);
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
    assert_eq!(locked_projects(dir.path())?, 2);
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

/// A `members` entry naming nothing refuses the lock outright.
///
/// `Workspace::load_with` refuses a member that is not there, and a
/// workspace lock reads every member.
#[test]
fn a_missing_member_refuses_the_lock() -> Result<(), Box<dyn std::error::Error>> {
    let dir =
        workspace("manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\", \"gone\"]\n")?;
    let member = dir.path().join("app");

    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains("member `gone` does not exist"));
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

/// A workspace root, canonicalized, with one member `app`.
///
/// `camino_tempfile::tempdir()` is `/var/folders/...` on macOS and a
/// short name on Windows, while everything the CLI prints comes back
/// canonical.
fn canonical_workspace(
    root_text: &str,
) -> Result<(camino_tempfile::Utf8TempDir, Utf8PathBuf), Box<dyn std::error::Error>> {
    let dir = workspace(root_text)?;
    let root = sysand_core::project::utils::wrapfs::canonicalize(dir.path())?;
    Ok((dir, root))
}

/// An outer workspace that does not `exclude` the inner one, which
/// workspaces.md "Config" rejects. Returns the inner member directory.
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

/// The same project, reached by `--dir` instead of by being in it.
///
/// A path given on the command line meets the same refusal as the
/// upward walk from the current directory: `sysand info --dir <stray>`
/// and `cd <stray> && sysand info` are two routes to one project and
/// must agree.
#[test]
fn info_at_an_unlisted_path_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    run_sysand_in(dir.path(), ["info", "--dir", "stray"], None)?
        .assert()
        .failure()
        .stderr(predicate::str::contains(UNLISTED_TEXT));
    Ok(())
}

#[test]
fn cloning_from_an_unlisted_path_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    run_sysand_in(
        dir.path(),
        ["clone", "--dir", "stray", "--target", "out"],
        None,
    )?
    .assert()
    .failure()
    .stderr(predicate::str::contains(UNLISTED_TEXT));
    Ok(())
}

/// `add` **writes**, so it must refuse before recording the usage in the
/// member's manifest: otherwise the `lock` that follows refuses the file
/// `add` has just written, leaving no way forward that does not involve
/// editing the manifest by hand.
#[test]
fn adding_a_dir_usage_to_an_unlisted_project_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    let member = dir.path().join("app");
    let before = std::fs::read_to_string(member.join("model.toml"))?;
    run_sysand_discovering(&member, ["add", "--dir", "../stray", "--no-lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(UNLISTED_TEXT));
    assert_eq!(
        std::fs::read_to_string(member.join("model.toml"))?,
        before,
        "and the manifest is untouched, which is what `--no-lock` hid"
    );
    Ok(())
}

/// A configured `[override]` reaching an unlisted project.
///
/// The overridden project is read with the workspace table, so an
/// override naming a **member** resolves rather than failing with "the
/// project is not a member of any workspace" about a project that is
/// listed in `members`, and an override naming an unlisted project under
/// the root is refused.
#[test]
fn an_override_naming_a_member_resolves() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    // `app` inherits nothing here, so this is the plain case; the
    // inheriting one is covered in core, where a fixture can be built
    // without a solve.
    write_config(dir.path(), "app")?;
    run_sysand_discovering(&dir.path().join("app"), ["lock"])?
        .assert()
        .success();
    Ok(())
}

#[test]
fn an_override_naming_an_unlisted_project_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    write_config(dir.path(), "stray")?;
    run_sysand_discovering(&dir.path().join("app"), ["lock"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(UNLISTED_TEXT));
    Ok(())
}

/// An **excluded** project is the case the refusal must not catch:
/// `workspaces.md` calls it "the only kind of non-member project that
/// may sit under a workspace root", so both routes read it.
#[test]
fn an_excluded_path_still_reads() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "local-dep")?;
    run_sysand_in(dir.path(), ["info", "--dir", "local-dep"], None)?
        .assert()
        .success();
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
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    )?;
    let excluded = dir.path().join("vendor/other");
    dependency(dir.path(), "vendor/other")?;
    // A dependency of its own, so the lockfile has a path to get wrong.
    dependency(dir.path(), "vendor/dep")?;
    // Written by hand: `add --as-local-src` records an override in
    // `sysand.toml` at the *config* root, which for an excluded project
    // is still the workspace root, so it cannot express this.
    std::fs::write(
        excluded.join(".project.json"),
        r#"{"name": "other", "publisher": "acme", "version": "1.0.0",
            "usage": [{"dir": "../dep", "publisher": "acme", "name": "local-dep"}]}"#,
    )?;

    run_sysand_discovering(&excluded, ["lock"])?
        .assert()
        .success();

    // Its own file, in its own directory.
    assert!(excluded.join(DEFAULT_LOCKFILE_NAME).exists());
    assert!(!dir.path().join(DEFAULT_LOCKFILE_NAME).exists());

    // And its contents are relative to *it*, not to the workspace. This
    // is what a `model_workspace_root` taken from the discovered table
    // rather than from `StartRegion` would get wrong: the table is
    // found here too, and an excluded project keeps its own state.
    let text = std::fs::read_to_string(excluded.join(DEFAULT_LOCKFILE_NAME))?;
    assert!(
        text.contains("../dep") && !text.contains("vendor/dep"),
        "source paths must be relative to the excluded project:\n{text}"
    );
    Ok(())
}

/// `lock` twice in a member still works, and writes one file -- at the
/// workspace root. The scope check reads the table and not the members,
/// which is why a member holding a lockfile of its own does not stop
/// anything either.
#[test]
fn a_members_own_lockfile_does_not_stop_a_later_command() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    assert!(dir.path().join(DEFAULT_LOCKFILE_NAME).exists());
    assert!(!member.join(DEFAULT_LOCKFILE_NAME).exists());
    run_sysand_discovering(&member, ["lock"])?
        .assert()
        .success();
    Ok(())
}

// ------------------------------------------------------- `print-root`

/// `workspaces.md` "Command scopes": the workspace root, from a member
/// as from the root.
#[test]
fn print_root_from_a_member_prints_the_workspace_root() -> Result<(), Box<dyn std::error::Error>> {
    let (dir, root) = canonical_workspace(ROOT)?;
    run_sysand_discovering(&dir.path().join("app"), ["print-root"])?
        .assert()
        .success()
        .stdout(format!("{root}\n"));
    Ok(())
}

/// The root defines no project, so a walk for a project goes past it.
#[test]
fn print_root_at_the_workspace_root_prints_it() -> Result<(), Box<dyn std::error::Error>> {
    let (dir, root) = canonical_workspace(ROOT)?;
    run_sysand_discovering(dir.path(), ["print-root"])?
        .assert()
        .success()
        .stdout(format!("{root}\n"));
    Ok(())
}

/// An excluded project is not in the workspace, so what it is inside of
/// is itself.
#[test]
fn print_root_in_an_excluded_project_prints_that_project() -> Result<(), Box<dyn std::error::Error>>
{
    let (dir, root) = canonical_workspace(
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    )?;
    dependency(dir.path(), "vendor/other")?;
    run_sysand_discovering(&dir.path().join("vendor/other"), ["print-root"])?
        .assert()
        .success()
        .stdout(format!("{}\n", root.join("vendor/other")));
    Ok(())
}

/// `print-root` has a scope, so an unlisted project is an error for it
/// too.
#[test]
fn print_root_in_an_unlisted_project_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    dependency(dir.path(), "stray")?;
    run_sysand_discovering(&dir.path().join("stray"), ["print-root"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(UNLISTED_TEXT));
    Ok(())
}

#[test]
fn print_root_outside_a_workspace_prints_the_project_root() -> Result<(), Box<dyn std::error::Error>>
{
    let (_temp_dir, cwd) = cli_init_json_project(None, "acme", "standalone", "1.2.3", None)?;
    run_sysand_discovering(&cwd, ["print-root"])?
        .assert()
        .success()
        .stdout(format!("{cwd}\n"));
    Ok(())
}

// ------------------- one lockfile and one env per workspace

/// A second member, at a depth, using the first. `dir` usages are
/// hand-written: `add` emits an IRI usage plus a config override, and a
/// group entry would be relative to the root, which the workspace lock
/// refuses.
fn two_member_workspace() -> Result<camino_tempfile::Utf8TempDir, Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\", \"libs/gamma\"]\n\
         exclude = [\"local-dep\"]\n",
    )?;
    std::fs::create_dir_all(dir.path().join("app"))?;
    std::fs::write(dir.path().join("app/model.toml"), MEMBER)?;
    std::fs::create_dir_all(dir.path().join("libs/gamma"))?;
    std::fs::write(
        dir.path().join("libs/gamma/model.toml"),
        "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\nname = \"gamma\"\n\
         version = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
         [[inline-usages]]\npublisher = \"acme\"\nname = \"alpha\"\ndir = \"../../app\"\n",
    )?;
    Ok(dir)
}

#[test]
fn lock_in_a_member_writes_at_the_workspace_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(&dir.path().join("app"), ["lock"])?
        .assert()
        .success();
    assert!(dir.path().join(DEFAULT_LOCKFILE_NAME).exists());
    assert!(!dir.path().join("app").join(DEFAULT_LOCKFILE_NAME).exists());
    Ok(())
}

/// Both members, and each recorded editable at its own path -- the
/// member at a depth reaching back two levels, which a single base
/// would resolve wrongly.
#[test]
fn the_workspace_lockfile_records_every_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(&dir.path().join("libs/gamma"), ["lock"])?
        .assert()
        .success();

    let text = std::fs::read_to_string(dir.path().join(DEFAULT_LOCKFILE_NAME))?;
    for expected in [
        "pkg:sysand/acme/alpha",
        "pkg:sysand/acme/gamma",
        "editable = \"app\"",
        "editable = \"libs/gamma\"",
    ] {
        assert!(
            text.contains(expected),
            "`{expected}` missing from:\n{text}"
        );
    }
    Ok(())
}

/// From the root, where there is no project at all -- which must not
/// bail with "not inside a project".
#[test]
fn lock_from_the_root_locks_every_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["lock"])?
        .assert()
        .success();
    assert_eq!(locked_projects(dir.path())?, 2);
    Ok(())
}

/// `sync` reads the root's lockfile and installs into the root's
/// `.sysand`. The hand-added comment survives, which a silent re-lock
/// would have removed.
#[test]
fn sync_in_a_member_uses_the_root_lockfile_and_env() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["lock"])?
        .assert()
        .success();
    let lockfile = dir.path().join(DEFAULT_LOCKFILE_NAME);
    let marked = format!("# kept\n{}", std::fs::read_to_string(&lockfile)?);
    std::fs::write(&lockfile, &marked)?;

    run_sysand_discovering(&dir.path().join("app"), ["sync"])?
        .assert()
        .success();

    assert!(
        dir.path().join(".sysand").is_dir(),
        "the env is at the root"
    );
    assert!(!dir.path().join("app/.sysand").exists());
    assert!(!dir.path().join("app").join(DEFAULT_LOCKFILE_NAME).exists());
    assert!(
        std::fs::read_to_string(&lockfile)?.starts_with("# kept"),
        "the lockfile was rewritten, so `sync` re-locked instead of reading"
    );
    Ok(())
}

/// The auto-lock fallback: no lockfile at all, and `sync` must lock the
/// workspace rather than one member into the current directory.
#[test]
fn sync_without_a_lockfile_locks_the_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(&dir.path().join("app"), ["sync"])?
        .assert()
        .success();
    assert_eq!(locked_projects(dir.path())?, 2);
    assert!(!dir.path().join("app").join(DEFAULT_LOCKFILE_NAME).exists());
    Ok(())
}

/// `add` in one member re-locks the workspace: the other member is
/// still there afterwards. Redirecting only the write would have left a
/// single-member solve over the shared file.
#[test]
fn add_in_a_member_keeps_every_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    dependency(dir.path(), "local-dep")?;
    run_sysand_discovering(
        &dir.path().join("app"),
        [
            "add",
            "--iri",
            USAGE,
            "--version-constraint",
            CONSTRAINT,
            "--as-local-src",
            "local-dep",
        ],
    )?
    .assert()
    .success();

    let text = std::fs::read_to_string(dir.path().join(DEFAULT_LOCKFILE_NAME))?;
    assert!(text.contains("pkg:sysand/acme/gamma"), "{text}");
    assert!(!dir.path().join("app").join(DEFAULT_LOCKFILE_NAME).exists());
    Ok(())
}

// ------------------- the selectors (Command scopes)

/// workspaces.md "Command scopes": "`--workspace` forces all members".
/// From a member directory the default is that member, so this is the
/// only place the flag changes anything.
#[test]
fn build_workspace_from_a_member_dir_builds_all() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(&dir.path().join("app"), ["build", "--workspace"])?
        .assert()
        .success();

    // One `output/`, at the root, holding both.
    assert!(dir.path().join("output/alpha-1.0.0.kpar").exists());
    assert!(dir.path().join("output/gamma-1.0.0.kpar").exists());
    Ok(())
}

/// "`--member <path-or-name>` picks one" -- by its `members` entry as
/// written.
#[test]
fn build_member_from_the_root_builds_one() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["build", "--member", "libs/gamma"])?
        .assert()
        .success();

    assert!(dir.path().join("output/gamma-1.0.0.kpar").exists());
    assert!(!dir.path().join("output/alpha-1.0.0.kpar").exists());
    Ok(())
}

/// And by the project's name, which is in the member's own manifest
/// rather than in the workspace table -- the reason a selector loads
/// the workspace where the rest of the CLI only consults it.
#[test]
fn a_member_selector_matches_a_name_too() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["build", "--member", "gamma"])?
        .assert()
        .success();

    assert!(dir.path().join("output/gamma-1.0.0.kpar").exists());
    assert!(!dir.path().join("output/alpha-1.0.0.kpar").exists());
    Ok(())
}

#[test]
fn a_member_selector_that_matches_nothing_says_so() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["build", "--member", "nosuch"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains("has no member `nosuch`"))
        // The message lists what there is, so the next attempt is informed.
        .stderr(predicate::str::contains("libs/gamma"));
    Ok(())
}

/// Outside a workspace a selector has nothing to select from, and is
/// refused rather than dropped: dropping it would edit the current
/// project instead of the one named.
#[test]
fn a_member_selector_outside_a_workspace_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(dir.path().join("model.toml"), MEMBER)?;
    run_sysand_discovering(dir.path(), ["add", "--member", "app", "acme/widgets"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains("`--member app` needs a workspace"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("model.toml"))?,
        MEMBER
    );
    Ok(())
}

/// The contradictory pair, refused by clap: a `Selector` cannot
/// express it.
#[test]
fn the_two_selectors_are_refused_together() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["build", "--workspace", "--member", "app"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the argument '--workspace' cannot be used with '--member <PATH-OR-NAME>'",
        ));
    Ok(())
}

/// A writing command run at the workspace root, which has no project of
/// its own, is refused with a message that names the remedy.
#[test]
fn a_writing_command_at_the_root_needs_a_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    for args in [
        vec!["add", "acme/dep", "--version-constraint", "^1"],
        vec!["remove", "acme/alpha"],
        vec!["include", "a.sysml"],
        vec!["exclude", "a.sysml"],
    ] {
        run_sysand_discovering(dir.path(), args.clone())?
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "this command needs a project; run it from a member directory or pass \
                 `--member <path-or-name>`",
            ));
    }
    Ok(())
}

/// And with the selector the message names, the same command reaches
/// the member.
#[test]
fn a_writing_command_at_the_root_with_member_works() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["remove", "--member", "gamma", "acme/alpha"])?
        .assert()
        .success();

    let manifest = std::fs::read_to_string(dir.path().join("libs/gamma/model.toml"))?;
    assert!(!manifest.contains("inline-usages"), "{manifest}");
    Ok(())
}

/// "`print-root` prints the workspace root ... `--member` the
/// member's".
#[test]
fn print_root_member_prints_the_members_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["print-root", "--member", "gamma"])?
        .assert()
        .success()
        .stdout(predicate::str::contains("libs/gamma"));
    Ok(())
}

/// "`env`: per workspace: the single `.sysand`". `sysand env` in a
/// member must not create a second one beside the shared environment.
#[test]
fn env_in_a_member_uses_the_root_sysand() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    run_sysand_discovering(&dir.path().join("app"), ["env"])?
        .assert()
        .success();

    assert!(dir.path().join(DEFAULT_ENV_NAME).is_dir());
    assert!(!dir.path().join("app").join(DEFAULT_ENV_NAME).exists());
    Ok(())
}

// ------------------- the root defaults for `sources` and `info`

/// A workspace whose members have sources, and whose first member uses
/// the second -- which workspaces.md calls the normal case and which
/// makes the second member's files reachable twice.
fn two_member_workspace_with_sources()
-> Result<camino_tempfile::Utf8TempDir, Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\", \"lib\"]\n",
    )?;
    for (at, name, extra) in [
        (
            "app",
            "app",
            "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\ndir = \"../lib\"\n",
        ),
        ("lib", "lib", ""),
    ] {
        std::fs::create_dir_all(dir.path().join(at))?;
        std::fs::write(
            dir.path().join(at).join("model.toml"),
            format!(
                "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\n\
                 name = \"{name}\"\nversion = \"1.0.0\"\nsources = [\"{at}.sysml\"]\n\
                 topics = [\"{at}-one\", \"{at}-two\"]\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n{extra}"
            ),
        )?;
        // A distinct symbol per member: two members exporting the same
        // one is a lockfile error, and a fixture should not be testing
        // that here.
        std::fs::write(
            dir.path().join(at).join(format!("{at}.sysml")),
            format!("package {};\n", name.to_uppercase()),
        )?;
    }
    Ok(dir)
}

/// workspaces.md "Command scopes": all members, from the root.
#[test]
fn sources_at_the_root_lists_every_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(dir.path(), ["sources", "--deps", "none"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;

    assert!(stdout.contains("app.sysml"), "{stdout}");
    assert!(stdout.contains("lib.sysml"), "{stdout}");
    Ok(())
}

/// The collision workspaces.md designs in: an intra-workspace usage
/// "resolves to the member itself, never to a copy of it from an
/// index", so `lib`'s own file is reached both as its own source and as
/// `app`'s dependency. It is listed once.
///
/// No dependency *ordering* is asserted: that order is not stable.
#[test]
fn sources_at_the_root_lists_a_member_dependency_once() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
        .assert()
        .success();
    run_sysand_discovering(dir.path(), ["sync", "--no-index"])?
        .assert()
        .success();

    let out = run_sysand_discovering(dir.path(), ["sources"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;

    assert_eq!(
        stdout.matches("lib.sysml").count(),
        1,
        "`lib`'s own source is also `app`'s dependency:\n{stdout}"
    );
    Ok(())
}

/// `--no-own` is "not the project's own sources". At a root one
/// member's dependency is another member's tree, so the members' own
/// sources come out of the union rather than being skipped per member.
#[test]
fn sources_no_own_at_the_root_excludes_member_sources() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
        .assert()
        .success();
    run_sysand_discovering(dir.path(), ["sync", "--no-index"])?
        .assert()
        .success();

    let out = run_sysand_discovering(dir.path(), ["sources", "--no-own"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    assert!(
        stdout.trim().is_empty(),
        "every source here is a member's own:\n{stdout}"
    );
    Ok(())
}

#[test]
fn sources_workspace_from_a_member_dir() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(
        &dir.path().join("app"),
        ["sources", "--workspace", "--deps", "none"],
    )?
    .assert()
    .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    assert!(stdout.contains("lib.sysml"), "{stdout}");
    Ok(())
}

/// One block per member, in `members` order. The blank line between
/// them is a visual separator and not a contract -- a `description` may
/// contain one -- but a block carries no trailing blank of its own, so
/// a one-member workspace prints what that member prints alone.
#[test]
fn info_at_the_root_prints_a_block_per_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(dir.path(), ["info"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;

    let app = stdout.find("Name: app").expect("app's block");
    let lib = stdout.find("Name: lib").expect("lib's block");
    assert!(app < lib, "`members` order:\n{stdout}");
    assert!(stdout.contains("\n\nName: lib"), "separated:\n{stdout}");
    Ok(())
}

/// A bare value per member says nothing about whose it is.
#[test]
fn info_a_field_at_the_root_names_each_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(dir.path(), ["info", "--get", "version"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    assert!(stdout.contains("app: 1.0.0"), "{stdout}");
    assert!(stdout.contains("lib: 1.0.0"), "{stdout}");
    Ok(())
}

/// The prefix is per *emitted line*, not per member: a list field
/// prints several.
#[test]
fn info_a_list_field_prefixes_every_line() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(dir.path(), ["info", "--get", "topic"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    for expected in ["app: app-one", "app: app-two", "lib: lib-one"] {
        assert!(stdout.contains(expected), "{expected} missing:\n{stdout}");
    }
    Ok(())
}

/// The single-project format is untouched.
#[test]
fn info_a_field_from_a_member_dir_is_bare() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_discovering(&dir.path().join("app"), ["info", "--get", "version"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    assert_eq!(stdout.trim(), "1.0.0");
    Ok(())
}

/// "All members" is a *reading* behaviour. Writing is `edit`, which
/// needs a project, so at the root it is refused like the other writing
/// commands.
#[test]
fn edit_at_the_root_needs_a_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    for args in [
        vec!["edit", "--version", "2.0.0"],
        vec!["edit", "--add-topic", "q"],
        vec!["edit", "--clear-license"],
    ] {
        run_sysand_discovering(dir.path(), args.clone())?
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "this command needs a project; run it from a member directory or pass \
                 `--member <path-or-name>`",
            ));
    }
    // And nothing was written.
    let manifest = std::fs::read_to_string(dir.path().join("app/model.toml"))?;
    assert!(manifest.contains("version = \"1.0.0\""), "{manifest}");
    Ok(())
}

/// A selector beside `--dir`/`--iri` is refused, not discarded in silence.
#[test]
fn a_selector_beside_an_explicit_target_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    run_sysand_discovering(dir.path(), ["info", "--member", "lib", "--dir", "app"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains("pass one or the other"));
    Ok(())
}

/// "From the workspace root **or from a non-member directory under it
/// that defines no project**, the whole workspace is the command's
/// target."
#[test]
fn a_non_member_directory_under_the_root_is_the_workspace() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = two_member_workspace_with_sources()?;
    std::fs::create_dir_all(dir.path().join("scratch"))?;

    let out = run_sysand_discovering(&dir.path().join("scratch"), ["info", "--get", "version"])?
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone())?;
    assert!(stdout.contains("app: 1.0.0"), "{stdout}");
    assert!(stdout.contains("lib: 1.0.0"), "{stdout}");
    Ok(())
}

// ------------------- stray per-member state

/// workspaces.md "Stray per-member state": "A member never has state of
/// its own: the lockfile, the `.sysand` directory and the `output`
/// directory all belong to the workspace root."
///
/// Enforced only where a workspace is *loaded*: the invariant is one the
/// commands that load members rely on.
#[test]
fn lock_refuses_a_member_holding_a_lockfile() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    std::fs::write(dir.path().join("app").join(DEFAULT_LOCKFILE_NAME), "")?;

    run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "member `app` has its own `sysand-lock.toml`; a workspace has one lockfile at",
        ))
        .stderr(predicate::str::contains("remove the member's copy"));
    Ok(())
}

/// All three kinds, across two members, in one run: the load reports
/// every broken member rather than stopping at the first.
#[test]
fn the_refusal_names_every_member_with_stray_state() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    std::fs::write(dir.path().join("app").join(DEFAULT_LOCKFILE_NAME), "")?;
    std::fs::create_dir_all(dir.path().join("app").join(DEFAULT_ENV_NAME))?;
    std::fs::create_dir_all(dir.path().join("libs/gamma").join("output"))?;

    run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "member `app` has its own `sysand-lock.toml`",
        ))
        .stderr(predicate::str::contains(
            "member `app` has its own `.sysand`",
        ))
        .stderr(predicate::str::contains(
            "member `libs/gamma` has its own `output`",
        ));
    Ok(())
}

/// The negative, which is what catches an over-eager check: a workspace
/// whose state is all at the root locks, syncs and builds repeatedly.
/// The commands write the very files a member may not hold -- at the
/// root -- so this is also the test that the second run of a command is
/// not an error.
#[test]
fn a_workspace_without_stray_state_is_unaffected() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    for _ in 0..2 {
        run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
            .assert()
            .success();
        run_sysand_discovering(dir.path(), ["sync", "--no-index"])?
            .assert()
            .success();
        run_sysand_discovering(dir.path(), ["build"])?
            .assert()
            .success();
    }
    // And from inside a member, where `build` must not leave an
    // `output/` behind.
    run_sysand_discovering(&dir.path().join("app"), ["build"])?
        .assert()
        .success();
    assert!(!dir.path().join("app").join("output").exists());
    assert!(!dir.path().join("app").join(DEFAULT_LOCKFILE_NAME).exists());
    Ok(())
}

/// The one way for a command to create stray per-member state: a
/// `--workspace` build whose output path lands inside a member would
/// write the state the next `lock` refuses. Refused before the
/// directory is created.
#[test]
fn build_workspace_refuses_an_output_path_inside_a_member() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = two_member_workspace()?;
    run_sysand_discovering(dir.path(), ["build", "--workspace", "app/output"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is inside the member `app`; a workspace has one build output directory",
        ));

    assert!(!dir.path().join("app").join("output").exists());
    // And the default is unaffected.
    run_sysand_discovering(dir.path(), ["build", "--workspace"])?
        .assert()
        .success();
    assert!(dir.path().join("output").is_dir());
    Ok(())
}

// ------------------- an inherited path usage

/// workspaces.md "Common usages": a group entry's `dir` "is relative to
/// the directory of the `model.toml` that defines the workspace [...]
/// whichever member opts into it". The KPAR view cannot carry that
/// base, and resolving against the member directory would silently pick
/// a different directory, so `build` refuses it, as `lock` does.
#[test]
fn build_refuses_a_member_inheriting_a_local_usage() -> Result<(), Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\n\n\
         [[workspace.inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n",
    )?;
    for (at, name) in [("app", "app"), ("dep", "dep")] {
        std::fs::create_dir_all(dir.path().join(at))?;
        std::fs::write(
            dir.path().join(at).join("model.toml"),
            format!(
                "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\n\
                 name = \"{name}\"\nversion = \"1.0.0\"\nsources = []\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
            ),
        )?;
    }
    // `app` opts into the group's `dir` usage.
    let app = dir.path().join("app").join("model.toml");
    let text = std::fs::read_to_string(&app)?;
    std::fs::write(
        &app,
        format!(
            "{text}\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nworkspace = true\n"
        ),
    )?;

    // From the member.
    run_sysand_discovering(&dir.path().join("app"), ["build"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is inherited from the workspace, and its path `dep` is relative",
        ))
        .stderr(predicate::str::contains("panicked").not());

    // And from the root, where the workspace build reports it per member.
    run_sysand_discovering(dir.path(), ["build"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains("panicked").not());
    Ok(())
}

/// The operations that build a resolver or read `[[index]]` warn about
/// a stray config, and `migrate` resolves through an index: a one-time
/// conversion is a bad place to be quiet about configuration that is
/// not read.
#[test]
fn migrate_warns_about_a_stray_config() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace(ROOT)?;
    let member = dir.path().join("app");
    write_config(&member, "local-dep")?;

    // A json project inside the member, so `migrate` has something to
    // convert and reaches the warning rather than an earlier refusal.
    let legacy = member.join("legacy");
    std::fs::create_dir_all(&legacy)?;
    std::fs::write(
        legacy.join(".project.json"),
        br#"{"name": "legacy", "publisher": "acme", "version": "1.0.0"}"#,
    )?;
    std::fs::write(
        legacy.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {},
            "metamodel": "https://www.omg.org/spec/SysML/20250201"}"#,
    )?;

    run_sysand_discovering(&member, ["migrate", "legacy", "--allow-no-vcs"])?
        .assert()
        .stderr(predicate::str::contains(format!(
            "`{}` {ROW_26}",
            member.join(config::local_fs::CONFIG_FILE)
        )));
    Ok(())
}

/// `design/model-toml.md`: the cycle check "runs over the dependency
/// graph whenever it is built — when a workspace is read, and when
/// resolving or building". A cyclic workspace that locked would get a
/// lockfile written for a graph with no build order.
#[test]
fn lock_refuses_a_cyclic_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"alpha\", \"beta\"]\n",
    )?;
    for (this, other) in [("alpha", "beta"), ("beta", "alpha")] {
        std::fs::create_dir_all(dir.path().join(this))?;
        std::fs::write(
            dir.path().join(this).join("model.toml"),
            format!(
                "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\n\
                 name = \"{this}\"\nversion = \"1.0.0\"\nsources = []\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
                 [[inline-usages]]\npublisher = \"acme\"\nname = \"{other}\"\n\
                 dir = \"../{other}\"\n"
            ),
        )?;
    }

    run_sysand_discovering(dir.path(), ["lock", "--no-index"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "dependency cycle: `alpha` -> `beta` -> `alpha`; only projects that carry `stdlib` \
             may form a cycle",
        ));

    assert!(
        !dir.path().join(DEFAULT_LOCKFILE_NAME).exists(),
        "no lockfile is written for a workspace that cannot be built"
    );
    Ok(())
}

/// `sources` names the cause, as `build` and `lock` do, rather than
/// the symptom "(base path unknown) not found".
#[test]
fn sources_refuses_a_member_inheriting_a_local_usage() -> Result<(), Box<dyn std::error::Error>> {
    let dir = camino_tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0.0\"\n\n[workspace]\nmembers = [\"app\"]\n\n\
         [[workspace.inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"dep\"\n",
    )?;
    for at in ["app", "dep"] {
        std::fs::create_dir_all(dir.path().join(at))?;
        std::fs::write(
            dir.path().join(at).join("model.toml"),
            format!(
                "manifest-version = \"1.0.0\"\n\n[project]\npublisher = \"acme\"\n\
                 name = \"{at}\"\nversion = \"1.0.0\"\nsources = []\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
            ),
        )?;
    }
    let app = dir.path().join("app").join("model.toml");
    let text = std::fs::read_to_string(&app)?;
    std::fs::write(
        &app,
        format!(
            "{text}\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nworkspace = true\n"
        ),
    )?;

    run_sysand_discovering(dir.path(), ["sources"])?
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "inherits the usage `acme/dep`, whose path `dep` is relative to the workspace root",
        ))
        .stderr(predicate::str::contains("base path unknown").not());
    Ok(())
}

/// workspaces.md "Command scopes", middle column: `sources` from a
/// member directory is that member.
///
/// The workspace has a *sibling* member, so a scope leak would show.
///
/// `--deps none` is load-bearing: `app` uses `lib`, so a plain
/// `sources` lists `lib.sysml` too -- as a dependency, not as a scope
/// leak. Without the flag this test would fail for a reason that has
/// nothing to do with scope.
#[test]
fn sources_from_a_member_dir_lists_that_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    let out = run_sysand_in(&dir.path().join("app"), ["sources", "--deps", "none"], None)?;
    out.clone().assert().success();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("app.sysml"), "got: {stdout}");
    assert!(
        !stdout.contains("lib.sysml"),
        "the sibling is out of scope: {stdout}"
    );
    Ok(())
}

/// The writing commands `remove`, `include` and `exclude` from a member
/// directory.
///
/// Each asserts its own effect, because the invariants differ: only
/// `remove` re-locks, and none of them writes the root `model.toml`, so
/// "the members list is intact" would be vacuous for all three.
#[test]
fn writing_commands_in_a_member_act_on_that_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    let member = dir.path().join("libs/gamma");
    let manifest = member.join("model.toml");

    // `include` first: `exclude` needs a source to remove, and this
    // member's `sources` is empty.
    std::fs::write(member.join("a.sysml"), "package A;\n")?;
    run_sysand_in(&member, ["include", "a.sysml"], None)?
        .assert()
        .success();
    assert!(
        std::fs::read_to_string(&manifest)?.contains("a.sysml"),
        "include writes the member's own manifest"
    );

    run_sysand_in(&member, ["exclude", "a.sysml"], None)?
        .assert()
        .success();
    assert!(
        !std::fs::read_to_string(&manifest)?.contains("a.sysml"),
        "exclude writes the member's own manifest"
    );

    // `remove` drops the usage and re-locks, at the workspace root.
    run_sysand_in(&member, ["remove", "acme/alpha", "--no-sync"], None)?
        .assert()
        .success();
    assert!(
        !std::fs::read_to_string(&manifest)?.contains("alpha"),
        "remove writes the member's own manifest"
    );
    assert!(
        !member.join("model.toml.bak").exists(),
        "no stray state in the member"
    );
    Ok(())
}

/// `edit` from a member directory writes that member.
/// `edit_at_the_root_needs_a_member` is the root half.
#[test]
fn edit_in_a_member_writes_that_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    let member = dir.path().join("libs/gamma");

    run_sysand_in(&member, ["edit", "--version", "2.0.0"], None)?
        .assert()
        .success();

    assert!(
        std::fs::read_to_string(member.join("model.toml"))?.contains("2.0.0"),
        "the member's own manifest is rewritten"
    );
    assert!(
        std::fs::read_to_string(dir.path().join("app/model.toml"))?.contains("1.0.0"),
        "the sibling is untouched"
    );
    Ok(())
}

/// `build` from a member directory builds that member, into the *root*
/// `output`.
///
/// The root `output` starts empty, so this tells "one" from "all", and
/// checks the table's "KPARs always land in the root `output`" for the
/// default scope.
#[test]
fn build_from_a_member_dir_builds_one_into_the_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace_with_sources()?;
    run_sysand_in(&dir.path().join("app"), ["build"], None)?
        .assert()
        .success();

    let built: Vec<String> = glob::glob(dir.path().join("output/*.kpar").as_str())?
        .filter_map(|p| {
            p.ok()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    assert_eq!(built.len(), 1, "exactly one member is built: {built:?}");
    assert!(built[0].starts_with("app-"), "and it is `app`: {built:?}");
    assert!(
        !dir.path().join("app/output").exists(),
        "the KPAR lands in the root `output`, not the member's"
    );
    Ok(())
}

/// An edit to any member's `model.toml` changes the root lockfile on the
/// next `lock`: the lockfile *follows* a member, which is the property
/// that makes one shared lockfile correct rather than merely tidy.
#[test]
fn editing_a_member_changes_the_root_lockfile() -> Result<(), Box<dyn std::error::Error>> {
    let dir = two_member_workspace()?;
    let lockfile = dir.path().join("sysand-lock.toml");

    run_sysand_in(dir.path(), ["lock", "--no-index"], None)?
        .assert()
        .success();
    let before = std::fs::read_to_string(&lockfile)?;
    assert!(before.contains("1.0.0"), "the member's version is in it");

    // The edit is a member's own manifest, not the root's.
    let member = dir.path().join("libs/gamma/model.toml");
    // `\nversion`, not `version`: a bare replace also hits
    // `manifest-version`, and the file then claims to be written for a
    // manifest version this Sysand does not support.
    let edited =
        std::fs::read_to_string(&member)?.replace("\nversion = \"1.0.0\"", "\nversion = \"2.5.0\"");
    std::fs::write(&member, edited)?;

    run_sysand_in(dir.path(), ["lock", "--no-index"], None)?
        .assert()
        .success();
    let after = std::fs::read_to_string(&lockfile)?;

    assert_ne!(before, after, "the root lockfile follows the member");
    assert!(
        after.contains("2.5.0"),
        "the member's new version reaches the root lockfile:\n{after}"
    );
    Ok(())
}
