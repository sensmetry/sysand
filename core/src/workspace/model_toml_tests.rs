// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;

use super::{
    DiscoverError, LoadError, LoadFailure, MemberChecks, PathRegion, SelectError, StartRegion,
    Workspace, discover_table, start_region,
};
use crate::manifest::diagnostics::{Diagnostic, Report};
use crate::project::utils::{ProjectDeserializationError, wrapfs};

const PROJECT: &str = "\
manifest-version = \"1.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

/// A workspace on disk: a root manifest and a tree of member
/// directories, which every test here needs together.
struct Fixture {
    dir: Utf8TempDir,
    /// Where the workspace root sits inside the temporary directory.
    /// Empty puts it at the top; a subdirectory leaves room for a
    /// project, or another workspace, *above* the root.
    at: &'static str,
}

impl Fixture {
    /// `root` is the body of the root `model.toml`.
    fn new(root: &str) -> Self {
        Self::at("", root)
    }

    fn at(sub: &'static str, root: &str) -> Self {
        let fixture = Self {
            dir: camino_tempfile::tempdir().expect("a temporary directory"),
            at: sub,
        };
        if !sub.is_empty() {
            std::fs::create_dir_all(fixture.dir.path().join(sub))
                .expect("create the workspace directory");
        }
        fixture.write("model.toml", root);
        fixture
    }

    /// The temporary directory, canonicalized: everything above the
    /// workspace root.
    fn base(&self) -> Utf8PathBuf {
        wrapfs::canonicalize(self.dir.path()).expect("the temporary directory resolves")
    }

    /// The root, canonicalized, which is what [`Workspace::load_with`]
    /// reports.
    fn root(&self) -> Utf8PathBuf {
        let base = self.base();
        if self.at.is_empty() {
            base
        } else {
            base.join(self.at)
        }
    }

    /// A path under the workspace root.
    fn under(&self, path: &str) -> Utf8PathBuf {
        self.root().join(path)
    }

    fn write(&self, path: &str, content: &str) {
        Self::write_at(&self.dir.path().join(self.at).join(path), content);
    }

    /// Write above the workspace root, where the temporary directory is.
    fn above(&self, path: &str, content: &str) {
        Self::write_at(&self.dir.path().join(path), content);
    }

    fn write_at(path: &Utf8Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent directories");
        }
        std::fs::write(path, content).expect("write a file");
    }

    fn mkdir(&self, path: &str) {
        std::fs::create_dir_all(self.dir.path().join(self.at).join(path))
            .expect("create a directory");
    }

    /// A member directory holding a valid `model.toml` project.
    fn project(&self, at: &str) {
        self.write(&format!("{at}/model.toml"), PROJECT);
    }

    /// The same, with a chosen project name; [`Fixture::project`] names every
    /// member `widgets`.
    fn project_named(&self, at: &str, name: &str) {
        self.write(
            &format!("{at}/model.toml"),
            &PROJECT.replace("name = \"widgets\"", &format!("name = \"{name}\"")),
        );
    }

    fn load(&self) -> Result<Workspace, LoadError> {
        self.load_with(MemberChecks::All)
    }

    fn load_with(&self, checks: MemberChecks) -> Result<Workspace, LoadError> {
        Workspace::load_with(self.dir.path().join(self.at), checks)
    }

    fn loaded(&self) -> Workspace {
        self.load().expect("the workspace loads")
    }

    fn failure(&self) -> LoadFailure {
        match self.load() {
            Err(LoadError::Invalid(failure)) => *failure,
            Err(other) => panic!("expected member problems, got `{other}`"),
            Ok(_) => panic!("expected the load to fail"),
        }
    }
}

// ----------------------------- two members that are one project

/// The lockfile holds one entry per identity, so two members claiming
/// one identity cannot be locked.
#[test]
fn two_members_that_name_one_project_are_refused() {
    let fixture = Fixture::new(&workspace_with("\"a\", \"b\""));
    fixture.project_named("a", "shared");
    fixture.project_named("b", "shared");

    let diagnostics = root_diagnostics(&fixture.failure()).to_vec();
    assert!(
        diagnostics.iter().any(|d| d
            .message
            .contains("members `a` and `b` name the same project `acme/shared`")),
        "{diagnostics:?}"
    );
}

/// Compared normalized: two entries whose identities normalize to the
/// same value are the same project.
#[test]
fn two_members_whose_identities_only_normalize_alike_are_refused() {
    let fixture = Fixture::new(&workspace_with("\"a\", \"b\""));
    fixture.write("a/model.toml", &PROJECT.replace("acme", "Acme Corp"));
    fixture.write("b/model.toml", &PROJECT.replace("acme", "acme-corp"));

    let diagnostics = root_diagnostics(&fixture.failure()).to_vec();
    assert!(
        diagnostics.iter().any(|d| d
            .message
            .contains("name the same project `acme-corp/widgets`")),
        "the normalized identity is what collides: {diagnostics:?}"
    );
}

/// Three members, one collision: the diagnostic names the two that
/// collide and not the one that does not.
#[test]
fn only_the_colliding_members_are_named() {
    let fixture = Fixture::new(&workspace_with("\"a\", \"b\", \"c\""));
    fixture.project_named("a", "shared");
    fixture.project_named("b", "distinct");
    fixture.project_named("c", "shared");

    let diagnostics = root_diagnostics(&fixture.failure()).to_vec();
    let collisions: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.contains("name the same project"))
        .collect();
    assert_eq!(collisions.len(), 1, "{diagnostics:?}");
    assert!(
        collisions[0].message.contains("`a` and `c`"),
        "{:?}",
        collisions[0]
    );
}

/// The guard against a check that fires on everything.
#[test]
fn members_that_are_different_projects_still_load() {
    let fixture = Fixture::new(&workspace_with("\"a\", \"b\""));
    fixture.project_named("a", "first");
    fixture.project_named("b", "second");

    assert_eq!(fixture.loaded().members().len(), 2);
}

/// The diagnostics raised while parsing the root's `[workspace]` table.
///
/// These come back as [`LoadError::Root`], not [`LoadError::Invalid`]:
/// the table never became a table, so there are no members to report
/// against.
fn table_diagnostics(fixture: &Fixture) -> Vec<Diagnostic> {
    match fixture.load() {
        Err(LoadError::Root(ProjectDeserializationError::Invalid { report })) => {
            report.diagnostics().to_vec()
        }
        Err(other) => panic!("expected root-table diagnostics, got `{other}`"),
        Ok(_) => panic!("expected the load to fail"),
    }
}

/// The diagnostics raised against the *root* manifest.
fn root_diagnostics(failure: &LoadFailure) -> &[Diagnostic] {
    root_report(failure).diagnostics()
}

/// The report about the *root* manifest.
fn root_report(failure: &LoadFailure) -> &Report {
    match failure.root.as_ref().expect("root diagnostics") {
        ProjectDeserializationError::Invalid { report } => report,
        other @ ProjectDeserializationError::Syntax { path: _, err: _ } => {
            panic!("expected an invalid-manifest error, got `{other}`")
        }
    }
}

fn only_root_message(failure: &LoadFailure) -> String {
    let diagnostics = root_diagnostics(failure);
    assert_eq!(
        diagnostics.len(),
        1,
        "expected exactly one diagnostic, got {diagnostics:?}"
    );
    diagnostics[0].message.clone()
}

fn workspace_with(members: &str) -> String {
    format!("manifest-version = \"1.0\"\n\n[workspace]\nmembers = [{members}]\n")
}

// ----------------------------------------------------------- loading

#[test]
fn a_workspace_loads_its_members() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    // Distinct projects: `Fixture::project` names them all `widgets`,
    // and two members that are one project are refused.
    fixture.project_named("app", "app-project");
    fixture.project_named("lib", "lib-project");

    let workspace = fixture.loaded();
    assert_eq!(workspace.root(), fixture.root());
    let names: Vec<&str> = workspace
        .members()
        .iter()
        .map(|member| member.path.raw.as_str())
        .collect();
    assert_eq!(names, ["app", "lib"]);
}

#[test]
fn a_workspace_with_no_members_loads() {
    let fixture = Fixture::new("manifest-version = \"1.0\"\n\n[workspace]\n");
    assert!(fixture.loaded().members().is_empty());
}

#[test]
fn a_member_path_is_resolved_against_the_root() {
    let fixture = Fixture::new(&workspace_with("\"nested/app\""));
    fixture.project("nested/app");

    let workspace = fixture.loaded();
    let member = &workspace.members()[0];
    assert_eq!(member.dir, fixture.root().join("nested").join("app"));
    assert_eq!(member.path.raw, "nested/app");
    assert_eq!(member.path.path.as_str(), "nested/app");
}

#[test]
fn a_member_that_is_not_there_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    let message = only_root_message(&fixture.failure());
    assert!(
        message.starts_with("member `app` does not exist; there is no directory at `"),
        "{message}"
    );
}

#[test]
fn a_member_that_is_a_file_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app", "not a directory\n");
    assert_eq!(
        only_root_message(&fixture.failure()),
        "member `app` is not a directory"
    );
}

#[test]
fn a_member_that_is_a_json_project_names_the_file_it_has() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/.project.json", "{}");
    fixture.write("app/.meta.json", "{}");
    assert_eq!(
        only_root_message(&fixture.failure()),
        "member `app` uses `.project.json`; a workspace member must use the `model.toml` project \
         definition (`sysand migrate app`)"
    );
}

/// The file that is *there* is named: a member holding only `.meta.json`
/// is told about `.meta.json`, not `.project.json`.
#[test]
fn a_member_with_only_a_meta_json_names_that() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/.meta.json", "{}");
    assert!(
        only_root_message(&fixture.failure()).starts_with("member `app` uses `.meta.json`;"),
        "the message must name the file the member actually has"
    );
}

#[test]
fn a_member_that_defines_no_project_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.mkdir("app");
    assert_eq!(
        only_root_message(&fixture.failure()),
        "member `app` defines no project"
    );
}

/// A member cannot be a workspace root: the two tables never share a file
/// and a member must define a project. `detect_format` cannot see it: the
/// directory holds a `model.toml` and nothing else.
#[test]
fn a_member_that_is_a_workspace_root_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write(
        "app/model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\n",
    );
    assert_eq!(
        only_root_message(&fixture.failure()),
        "member `app` defines a workspace, not a project; a member cannot be a workspace root"
    );
}

#[test]
fn a_member_holding_both_manifests_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/.project.json", "{}");

    let message = only_root_message(&fixture.failure());
    // The clause is written out in this module; this holds it together
    // with the error that owns it.
    let owned = crate::project::local_src::LocalSrcError::AmbiguousFormat {
        dir: Utf8PathBuf::from("app"),
        json: ".project.json",
    }
    .to_string();
    // The owned message wraps and indents its second clause; compare
    // the words.
    //
    // The member's message appends a remedy the owned one cannot give
    // (`AmbiguousFormat` is raised for standalone projects too), so the
    // remedy is stripped before the comparison.
    let hint = "; remove the one that does not define the project";
    let shared = message
        .strip_suffix(hint)
        .expect("the member's message ends with its remedy");
    let words = |text: &str| {
        text.split_once(';')
            .expect("the ambiguity message has two clauses")
            .1
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(words(shared), words(&owned));
    assert!(message.starts_with("member `app` has both `model.toml` and `.project.json`;"));
}

#[test]
fn every_bad_member_is_reported_not_just_the_first() {
    let fixture = Fixture::new(&workspace_with("\"a\", \"b\", \"c\""));
    fixture.mkdir("a");
    fixture.write("b/.project.json", "{}");
    fixture.project("c");
    fixture.write("c/sysand-lock.toml", "");

    let failure = fixture.failure();
    assert_eq!(root_diagnostics(&failure).len(), 3);
}

#[test]
fn a_member_with_its_own_lockfile_names_the_root() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/sysand-lock.toml", "");
    assert_eq!(
        only_root_message(&fixture.failure()),
        format!(
            "member `app` has its own `sysand-lock.toml`; a workspace has one lockfile at `{}`; \
             remove the member's copy",
            fixture.root()
        )
    );
}

#[test]
fn a_member_with_its_own_sysand_directory_names_the_root() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("app/.sysand");
    assert_eq!(
        only_root_message(&fixture.failure()),
        format!(
            "member `app` has its own `.sysand`; a workspace has one environment at `{}`; remove \
             the member's copy",
            fixture.root()
        )
    );
}

#[test]
fn a_member_with_an_output_directory_names_the_root() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("app/output");
    assert_eq!(
        only_root_message(&fixture.failure()),
        format!(
            "member `app` has its own `output`; a workspace has one build output directory at \
             `{}`; remove the member's copy",
            fixture.root()
        )
    );
}

/// Standalone projects under an `exclude`d directory keep their state
/// legitimately; the check applies to members only.
#[test]
fn an_excluded_project_keeps_its_own_state() {
    // The state sits on the `exclude` entry itself, so that "run the
    // member checks over `exclude` too" is a mutation this test catches.
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.project("vendor");
    fixture.write("vendor/sysand-lock.toml", "");
    fixture.mkdir("vendor/.sysand");
    fixture.mkdir("vendor/output");

    assert_eq!(fixture.loaded().members().len(), 1);
}

/// `exclude` says what is *outside*. Nothing needs to be there at all,
/// let alone a project.
#[test]
fn an_excluded_directory_need_not_exist_or_hold_a_project() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         exclude = [\"vendor\", \"gone\"]\n",
    );
    fixture.project("app");
    fixture.mkdir("vendor");

    assert_eq!(fixture.loaded().members().len(), 1);
}

#[test]
fn a_member_diagnostic_points_at_the_entry_that_named_it() {
    let fixture =
        Fixture::new("manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\n  \"app\",\n]\n");
    let failure = fixture.failure();
    let report = root_report(&failure);
    assert_eq!(report.file(), "model.toml");
    let position = report.diagnostics()[0]
        .position
        .as_ref()
        .expect("the diagnostic points at the entry");
    assert_eq!(position.line, 5);
}

/// A path is where it points and how it was spelled; where it was
/// *written* is provenance for a message.
#[test]
fn a_member_position_does_not_change_path_equality() {
    use crate::manifest::diagnostics::Position;
    use crate::manifest::workspace_table::MemberPath;

    let path = crate::paths::normalize_relative("app").expect("a valid path");
    let without = MemberPath {
        raw: "app".to_owned(),
        path: path.clone(),
        at: None,
    };
    let with = MemberPath {
        raw: "app".to_owned(),
        path,
        at: Some(Position::at(5, 3)),
    };
    assert_eq!(without, with);
}

#[test]
fn a_root_that_is_not_a_workspace_is_refused() {
    let fixture = Fixture::new(PROJECT);
    match fixture.load() {
        Err(LoadError::NotAWorkspace(root)) => assert_eq!(root, fixture.root()),
        other => panic!("expected `NotAWorkspace`, got `{other:?}`"),
    }
}

#[test]
fn a_root_with_no_manifest_is_refused() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    assert_matches!(
        Workspace::load_with(dir.path(), MemberChecks::All),
        Err(LoadError::Io(_))
    );
}

// ------------------------------------ inheritance and classification

const INHERITING: &str = "\
manifest-version = \"1.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version.workspace = true
sources = []
metamodel = \"https://www.omg.org/spec/SysML/20250201\"
";

/// A member's own marker is followed by resolving it against the table
/// at load.
#[test]
fn a_member_inherits_from_the_table() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nversion = \"9.9.9\"\n",
    );
    fixture.write("app/model.toml", INHERITING);

    let workspace = fixture.loaded();
    assert_eq!(workspace.members()[0].project.version.to_string(), "9.9.9");
}

#[test]
fn a_member_whose_own_manifest_is_broken_is_named_by_its_own_file() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write(
        "app/model.toml",
        "manifest-version = \"1.0\"\n\n[project]\n",
    );

    let failure = fixture.failure();
    assert!(failure.root.is_none(), "the root manifest is fine");
    assert_eq!(failure.members.len(), 1);
    let text = failure.members[0].to_string();
    assert!(
        text.contains("app/model.toml"),
        "the member's own file must be named, got `{text}`"
    );
}

#[test]
fn a_member_whose_markers_do_not_resolve_is_named_by_its_own_file() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/model.toml", INHERITING);

    let failure = fixture.failure();
    assert!(failure.root.is_none());
    assert_eq!(failure.members.len(), 1);
    let text = failure.members[0].to_string();
    assert!(text.contains("app/model.toml"), "{text}");
}

fn classified() -> (Fixture, Workspace) {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\n\
         members = [\"app\"]\n\
         exclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.write("vendor/dep/model.toml", PROJECT);
    let workspace = fixture.loaded();
    (fixture, workspace)
}

/// The region of `path`, relative to the root, in the loaded table.
fn classify<'a>(workspace: &'a Workspace, path: &str) -> Region<'a> {
    workspace.table().region(Utf8UnixPath::new(path))
}

#[test]
fn a_listed_directory_is_a_member() {
    let (_fixture, workspace) = classified();
    match classify(&workspace, "app") {
        Region::Member(_, member) => assert_eq!(member.raw, "app"),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

#[test]
fn an_excluded_directory_is_excluded() {
    let (_fixture, workspace) = classified();
    match classify(&workspace, "vendor") {
        Region::Excluded(path) => assert_eq!(path.raw, "vendor"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

/// Anything under an `exclude`d directory is outside the workspace.
#[test]
fn a_directory_under_an_excluded_one_is_excluded() {
    let (_fixture, workspace) = classified();
    assert_matches!(
        classify(&workspace, "vendor/other/deep"),
        Region::Excluded(_)
    );
}

/// A directory listed in both `members` and `exclude` is refused.
#[test]
fn a_directory_in_both_lists_is_refused() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\n\
         members = [\"app\"]\n\
         exclude = [\"app\"]\n",
    );
    fixture.project("app");
    let diagnostics = table_diagnostics(&fixture);
    assert!(
        diagnostics.iter().any(|d| d
            .message
            .contains("`members` path `app` is also in `exclude`")),
        "the two lists say opposite things about one directory: {diagnostics:?}"
    );
}

/// A member listed *inside* an excluded directory is refused.
///
/// Anything under an `exclude`d directory is outside the workspace, so
/// the two lists make contradictory claims about one directory.
#[test]
fn a_member_under_an_excluded_directory_is_refused() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\n\
         members = [\"vendor/kept\"]\n\
         exclude = [\"vendor\"]\n",
    );
    fixture.project("vendor/kept");
    let diagnostics = table_diagnostics(&fixture);
    assert!(
        diagnostics.iter().any(|d| d
            .message
            .contains("`members` path `vendor/kept` lies under `exclude` path `vendor`")),
        "everything under an excluded directory is outside the workspace: {diagnostics:?}"
    );
}

/// The reverse nesting is *not* a conflict and must keep loading: a
/// project nested inside a member is not a member, so excluding a
/// vendored subtree inside a member contradicts nothing.
#[test]
fn an_excluded_directory_inside_a_member_still_loads() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\n\
         members = [\"app\"]\n\
         exclude = [\"app/sub\"]\n",
    );
    fixture.project("app");
    fixture.write("app/sub/model.toml", PROJECT);
    let workspace = fixture.loaded();
    assert_eq!(workspace.members().len(), 1);
}

/// Under an excluded directory a workspace root is a legitimate, separate
/// workspace.
#[test]
fn a_workspace_root_under_an_excluded_directory_is_excluded() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.write(
        "vendor/inner/model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\n",
    );
    let workspace = fixture.loaded();
    assert_matches!(classify(&workspace, "vendor/inner"), Region::Excluded(_));
}

/// `exclude` entries may nest, so the most specific one is the one that
/// answers.
#[test]
fn the_most_specific_excluded_entry_answers() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         exclude = [\"vendor\", \"vendor/deep\"]\n",
    );
    fixture.project("app");
    let workspace = fixture.loaded();
    match classify(&workspace, "vendor/deep/inner") {
        Region::Excluded(path) => assert_eq!(path.raw, "vendor/deep"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

/// The specificity rule runs both ways: an `exclude` entry *inside* a
/// member carves that subtree back out.
#[test]
fn an_excluded_directory_inside_a_member_is_excluded() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"app/gen\"]\n",
    );
    fixture.project("app");
    let workspace = fixture.loaded();
    assert_matches!(classify(&workspace, "app/src"), Region::Member(0, _));
    match classify(&workspace, "app/gen/out") {
        Region::Excluded(path) => assert_eq!(path.raw, "app/gen"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

#[test]
fn anything_else_is_unlisted() {
    let (_fixture, workspace) = classified();
    assert_matches!(classify(&workspace, "elsewhere"), Region::Unlisted);
    assert_matches!(classify(&workspace, ""), Region::Unlisted);
}

#[test]
fn a_directory_outside_the_root_is_outside() {
    let (_fixture, workspace) = classified();
    let outside = workspace.root().parent().expect("the root has a parent");
    assert_eq!(
        super::region_of(workspace.root(), workspace.table(), outside),
        PathRegion::Outside
    );
}

/// `app2` starts with `app`, but is not under it.
#[test]
fn classification_is_by_whole_components() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let workspace = fixture.loaded();

    assert_matches!(classify(&workspace, "app"), Region::Member(0, _));
    assert_matches!(classify(&workspace, "app2"), Region::Unlisted);
}

// --------------------------------------------------- the upward walk

/// The workspace root discovery from `from` settles on.
fn found_root(from: impl AsRef<Utf8Path>) -> Result<Option<Utf8PathBuf>, DiscoverError> {
    Ok(discover_table(from)?.map(|(root, _)| root))
}

/// Discovery from `candidate`, a workspace root, for its nesting verdict.
fn confirm(candidate: impl AsRef<Utf8Path>) -> Result<(), DiscoverError> {
    discover_table(candidate).map(|_| ())
}

const EMPTY_WORKSPACE: &str = "manifest-version = \"1.0\"\n\n[workspace]\n";

#[test]
fn a_workspace_is_found_in_the_starting_directory() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    assert_eq!(
        found_root(fixture.root()).expect("readable"),
        Some(fixture.root())
    );
}

#[test]
fn a_workspace_is_found_in_an_ancestor() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.mkdir("app/src/deep");
    assert_eq!(
        found_root(fixture.under("app/src/deep")).expect("readable"),
        Some(fixture.root())
    );
}

/// Not hermetic, and cannot be: the walk runs to the filesystem root.
/// A `model.toml` in an ancestor of the temporary directory would fail
/// this.
#[test]
fn no_workspace_above_the_starting_directory_is_none() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    assert_eq!(found_root(dir.path()).expect("readable"), None);
}

/// A member's directory holds a `[project]`, which is not a root.
#[test]
fn the_walk_goes_past_a_project() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    assert_eq!(
        found_root(fixture.under("app")).expect("readable"),
        Some(fixture.root())
    );
}

/// The file names neither table.
#[test]
fn the_walk_goes_past_a_manifest_with_neither_table() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    fixture.write("odd/model.toml", "manifest-version = \"1.0\"\n");
    assert_eq!(
        found_root(fixture.under("odd")).expect("readable"),
        Some(fixture.root())
    );
}

/// Walking past would answer *no workspace* for a directory whose
/// manifest may well be one.
#[test]
fn a_manifest_that_is_not_toml_is_reported() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    fixture.write("broken/model.toml", "this is not toml\n");
    match found_root(fixture.under("broken")) {
        Err(DiscoverError::Manifest(error)) => {
            let text = error.to_string();
            let manifest = Utf8Path::new("broken").join("model.toml");
            assert!(text.contains(manifest.as_str()), "{text}");
        }
        other => panic!("expected a manifest error, got `{other:?}`"),
    }
}

/// The walk climbs with `Utf8Path::parent`, which is textual. A path that
/// detours through another directory must still find the right workspace
/// and hand back a root without a `..` in it.
#[test]
fn the_root_comes_back_canonical() {
    let fixture = Fixture::at("ws", &workspace_with("\"app\""));
    fixture.project("app");
    fixture.above("detour/.keep", "");
    let roundabout = fixture
        .base()
        .join("detour")
        .join("..")
        .join("ws")
        .join("app");

    assert_eq!(
        found_root(roundabout).expect("readable"),
        Some(fixture.root())
    );
}

#[test]
fn a_nested_workspace_is_refused() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);

    match confirm(fixture.root()) {
        Err(error @ DiscoverError::Nested { inner: _, outer: _ }) => assert_eq!(
            error.to_string(),
            format!(
                "the workspace at `{}` lies inside the workspace at `{}`, which does not \
                 `exclude` it; nested workspaces are not supported",
                fixture.root(),
                fixture.base()
            )
        ),
        other => panic!("expected nesting, got `{other:?}`"),
    }
}

#[test]
fn a_nested_workspace_the_outer_excludes_is_confirmed() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\nexclude = [\"inner\"]\n",
    );
    confirm(fixture.root()).expect("the outer excludes it");
}

/// The outer workspace may list a directory above the candidate in
/// `exclude`.
#[test]
fn a_nested_workspace_excluded_by_a_directory_above_it_is_confirmed() {
    let fixture = Fixture::at("vendor/inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\nexclude = [\"vendor\"]\n",
    );
    confirm(fixture.root()).expect("an excluded directory above it");
}

/// The first outer settles the question either way; a third workspace
/// above it is not this command's problem.
#[test]
fn the_outer_workspace_is_not_itself_checked_for_nesting() {
    let fixture = Fixture::at("b/c", EMPTY_WORKSPACE);
    fixture.above(
        "b/model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\nexclude = [\"c\"]\n",
    );
    fixture.above("model.toml", EMPTY_WORKSPACE);
    confirm(fixture.root()).expect("`b` excludes `c`; `a` is `b`'s problem");
}

#[test]
fn an_outer_workspace_far_above_is_still_found() {
    let fixture = Fixture::at("a/b/c/inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);
    assert_matches!(
        confirm(fixture.root()),
        Err(DiscoverError::Nested { inner: _, outer: _ })
    );
}

/// `parse` runs the table's own validation, and its report is
/// all-or-nothing: a table whose `exclude` does not parse cannot answer
/// the question being asked of it.
#[test]
fn an_outer_whose_member_paths_do_not_parse_is_reported() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"/absolute\"]\n",
    );
    assert_matches!(confirm(fixture.root()), Err(DiscoverError::Manifest(_)));
}

/// Parsing is enough to answer the exclusion question; loading is not.
/// A command inside a separate workspace must not fail because a
/// neighbour's member directory is missing.
#[test]
fn an_outer_whose_member_is_broken_still_confirms_the_candidate() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"gone\"]\nexclude = [\"inner\"]\n",
    );
    confirm(fixture.root()).expect("the outer's own members are not this command's problem");
}

// ---------------------------------------------------- the starting region

/// Where a command run in `from` starts, over the table discovery finds.
fn start(fixture: &Fixture, from: impl AsRef<Utf8Path>) -> Result<StartRegion, DiscoverError> {
    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    start_region(&root, &table, from)
}

#[test]
fn a_member_directory_starts_in_that_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    assert_eq!(
        start(&fixture, fixture.under("app")).expect("classifies"),
        StartRegion::Member(0)
    );
}

#[test]
fn a_directory_under_a_member_starts_in_that_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("app/src/deep");
    assert_eq!(
        start(&fixture, fixture.under("app/src/deep")).expect("classifies"),
        StartRegion::Member(0)
    );
}

/// An excluded project is the only kind of non-member project that may
/// sit under a workspace root.
#[test]
fn an_excluded_project_starts_standalone() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.project("vendor/other");
    assert_eq!(
        start(&fixture, fixture.under("vendor/other")).expect("classifies"),
        StartRegion::Standalone(fixture.under("vendor/other"))
    );
}

#[test]
fn an_unlisted_project_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.project("stray");
    let error = start(&fixture, fixture.under("stray")).expect_err("an unlisted project");
    assert_eq!(
        error.to_string(),
        format!(
            "the project at `{}` lies under the workspace at `{}` but is neither listed in \
             `workspace.members` nor under `workspace.exclude`; add it to one of the two",
            fixture.under("stray"),
            fixture.root()
        )
    );
}

#[test]
fn a_directory_with_no_project_starts_in_the_workspace() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("notes");
    assert_eq!(
        start(&fixture, fixture.under("notes")).expect("classifies"),
        StartRegion::Workspace
    );
}

/// The workspace root is never a member, so commands run from it always
/// target the whole workspace.
#[test]
fn the_root_starts_in_the_workspace() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    assert_eq!(
        start(&fixture, fixture.root()).expect("classifies"),
        StartRegion::Workspace
    );
}

/// A project above the root is a different project, which this
/// workspace has no opinion about.
#[test]
fn a_project_above_the_root_is_not_the_starting_project() {
    let fixture = Fixture::at("ws", &workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("notes");
    fixture.above("model.toml", PROJECT);
    assert_eq!(
        start(&fixture, fixture.under("notes")).expect("classifies"),
        StartRegion::Workspace
    );
}

#[test]
fn a_directory_outside_the_workspace_is_refused() {
    let fixture = Fixture::at("ws", &workspace_with("\"app\""));
    fixture.project("app");
    assert_matches!(
        start(&fixture, fixture.base()),
        Err(DiscoverError::NotUnderRoot { dir: _, root: _ })
    );
}

/// `relativize_path` asserts absolute, dot-free paths with a plain
/// `assert!`, which panics in release too.
///
/// A `..` component and not a `.` one: `Utf8Path::components` drops `.`,
/// so a path spelled with one is already canonical.
#[test]
fn a_starting_path_with_a_parent_component_is_accepted() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let roundabout = fixture.under("app").join("..").join("app");

    let (root, table) = discover_table(&roundabout)
        .expect("readable")
        .expect("a workspace");
    assert_eq!(root, fixture.root());
    assert_eq!(
        start_region(&root, &table, &roundabout).expect("classifies"),
        StartRegion::Member(0)
    );
}

// ----------------------------------------------------------- selection

fn two_members() -> Fixture {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    fixture.project_named("app", "alpha");
    fixture.project_named("lib", "beta");
    fixture
}

#[test]
fn a_member_is_selected_by_its_path() {
    let workspace = two_members().loaded();
    assert_eq!(workspace.member("lib").expect("a member").path.raw, "lib");
}

/// A path matches the spelling the author wrote, which is what
/// `MemberPath::raw` is kept for: another spelling of the same directory
/// selects nothing.
#[test]
fn a_member_path_is_matched_as_written() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project_named("app", "alpha");
    let workspace = fixture.loaded();

    assert_eq!(workspace.member("app").expect("a member").path.raw, "app");
    assert_matches!(
        workspace.member("./app/"),
        Err(SelectError::NoSuchMember {
            root: _,
            selector: _,
            known: _
        })
    );
}

#[test]
fn a_member_is_selected_by_its_project_name() {
    let workspace = two_members().loaded();
    assert_eq!(workspace.member("beta").expect("a member").path.raw, "lib");
}

/// The case that makes the rule a union: guessing would act on the wrong
/// project.
#[test]
fn a_value_that_is_one_members_path_and_anothers_name_is_ambiguous() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    fixture.project_named("app", "alpha");
    fixture.project_named("lib", "app");
    let workspace = fixture.loaded();

    match workspace.member("app") {
        Err(
            error @ SelectError::AmbiguousMember {
                selector: _,
                matches: _,
            },
        ) => assert_eq!(
            error.to_string(),
            "`app` is ambiguous: it matches the path `app` and the name of the member at `lib`"
        ),
        other => panic!("expected ambiguity, got `{other:?}`"),
    }
}

#[test]
fn two_members_with_the_same_name_are_ambiguous() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    // Two members may share a *name* while being different projects,
    // which is what makes the selector ambiguous. They may not share an
    // identity, so the two have different publishers.
    fixture.write(
        "app/model.toml",
        &PROJECT.replace("widgets", "alpha").replace("acme", "first"),
    );
    fixture.write(
        "lib/model.toml",
        &PROJECT
            .replace("widgets", "alpha")
            .replace("acme", "second"),
    );
    let workspace = fixture.loaded();

    assert_matches!(
        workspace.member("alpha"),
        Err(SelectError::AmbiguousMember {
            selector: _,
            matches: _
        })
    );
}

/// A member the selector names twice is still one member.
#[test]
fn a_member_named_by_both_its_path_and_its_name_is_not_ambiguous() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    fixture.project_named("app", "app");
    fixture.project_named("lib", "beta");
    let workspace = fixture.loaded();

    assert_eq!(workspace.member("app").expect("one member").path.raw, "app");
}

#[test]
fn an_unknown_member_lists_the_ones_that_exist() {
    let workspace = two_members().loaded();
    match workspace.member("nope") {
        Err(
            error @ SelectError::NoSuchMember {
                root: _,
                selector: _,
                known: _,
            },
        ) => {
            let text = error.to_string();
            assert!(
                text.ends_with("has no member `nope`; it has `app`, `lib`"),
                "{text}"
            );
        }
        other => panic!("expected no such member, got `{other:?}`"),
    }
}

// ------------------------------------------------------ table regions

use crate::manifest::workspace_table::{Region, WorkspaceTable};
use typed_path::Utf8UnixPath;

/// A table with every shape a valid table can hold: a plain member, a
/// plain exclusion, an exclusion inside a member, and an exclusion that
/// covers nothing.
///
/// A member inside an exclusion and a path written in both lists are
/// refused by the cross-list check, so they cannot appear here.
/// [`WorkspaceTable::region`] weighs member against exclusion by depth,
/// which the `app`/`app/gen` pair exercises.
const SHAPES: &str = "\
manifest-version = \"1.0\"

[workspace]
members = [\"app\"]
exclude = [\"vendor\", \"app/gen\", \"deep/a/b\"]
";

fn shapes() -> WorkspaceTable {
    let fixture = Fixture::new(SHAPES);
    fixture.project_named("app", "alpha");
    let (_root, table) = discover_table(fixture.root())
        .expect("readable")
        .expect("a workspace");
    table
}

fn region_of<'a>(table: &'a WorkspaceTable, path: &str) -> Region<'a> {
    table.region(Utf8UnixPath::new(path))
}

#[test]
fn a_listed_path_is_a_member_region() {
    let table = shapes();
    match region_of(&table, "app") {
        Region::Member(index, entry) => {
            assert_eq!(index, 0);
            assert_eq!(entry.raw, "app");
        }
        other => panic!("expected a member, got `{other:?}`"),
    }
}

#[test]
fn an_excluded_path_is_an_excluded_region() {
    let table = shapes();
    match region_of(&table, "vendor/other") {
        Region::Excluded(entry) => assert_eq!(entry.raw, "vendor"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

#[test]
fn an_unknown_path_is_unlisted() {
    let table = shapes();
    assert_matches!(region_of(&table, "notes"), Region::Unlisted);
    assert_matches!(region_of(&table, "deep/a"), Region::Unlisted);
}

/// The one combination that survives the cross-list check: an
/// `exclude` inside a member, which carves that subtree back out.
#[test]
fn a_region_is_decided_by_the_most_specific_entry() {
    let table = shapes();

    assert_matches!(region_of(&table, "app/src"), Region::Member(0, _));
    match region_of(&table, "app/gen/out") {
        Region::Excluded(entry) => assert_eq!(entry.raw, "app/gen"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

/// Creating a new project cannot be blocked by an unrelated member's
/// mistake.
#[test]
fn a_table_is_discovered_without_loading_its_members() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"gone\""));
    fixture.project_named("app", "alpha");

    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(root, fixture.root());
    assert_eq!(table.members.len(), 2);

    assert!(
        fixture.load().is_err(),
        "loading the same workspace must still refuse it"
    );
}

#[test]
fn a_discovered_table_is_still_checked_for_nesting() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);
    assert_matches!(
        discover_table(fixture.root()),
        Err(DiscoverError::Nested { inner: _, outer: _ })
    );
}

/// The candidate is parsed before the nesting verdict, so a file
/// carrying both tables fails as that.
#[test]
fn a_candidate_defining_both_tables_still_fails_as_that() {
    let fixture = Fixture::at("inner", &format!("{EMPTY_WORKSPACE}\n{PROJECT}"));
    fixture.above("model.toml", EMPTY_WORKSPACE);
    assert_matches!(
        discover_table(fixture.root()),
        Err(DiscoverError::Manifest(_))
    );
}

// ------------------------------- `start_region` reads no member

/// The property the CLI depends on: the walk answers over the table
/// alone, so a member that `Workspace::load` refuses does not stop it.
///
/// A member's own `sysand-lock.toml` is an error, and the walk must still answer for a
/// member that has one.
#[test]
fn start_region_ignores_a_members_own_lockfile() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/sysand-lock.toml", "");
    fixture
        .load()
        .expect_err("the tree is one `load` refuses, which is the point");

    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(
        start_region(&root, &table, fixture.under("app")).expect("classifies"),
        StartRegion::Member(0)
    );
}

/// And the same for an entry naming nothing: only the entry the walk
/// matched is consulted.
#[test]
fn start_region_ignores_a_member_that_does_not_exist() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"gone\""));
    fixture.project("app");
    fixture.load().expect_err("`load` refuses a missing member");

    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(
        super::start_region(&root, &table, fixture.under("app")).expect("classifies"),
        super::StartRegion::Member(0)
    );
}

// ------------------------------- `region_of` asks for identity

/// `app` is a member, and the project at `app/dep`, which
/// [`WorkspaceTable::region`] prefix-matches to the same entry, is not.
///
/// The consequence is inheritance: the nested project's
/// `publisher.workspace = true` must not resolve against the table.
#[test]
fn a_project_nested_inside_a_member_is_no_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.project("app/dep");

    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");

    assert_eq!(
        super::region_of(&root, &table, &fixture.under("app")),
        PathRegion::Member(0),
        "`app` is the entry"
    );
    assert_eq!(
        super::region_of(&root, &table, &fixture.under("app/dep")),
        PathRegion::Unlisted,
        "`app/dep` is inside the entry, which is not the same thing"
    );
    // A prefix match would answer `Member(0)` here, so a command run
    // inside `app/dep` would behave as though it were `app` while
    // `--path app/dep` is refused. Both are refused, as is any project
    // under the root that is neither listed nor excluded.
    assert_matches!(
        super::start_region(&root, &table, fixture.under("app/dep")),
        Err(super::DiscoverError::Unlisted {
            project: _,
            root: _
        }),
        "a project inside a member is not that member"
    );
}

/// The root defines no project, so it is no member either, and it is the
/// one path whose relativization is empty.
#[test]
fn the_root_is_no_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");

    let (root, table) = discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(
        super::region_of(&root, &table, &fixture.root()),
        PathRegion::Unlisted
    );
}

// ------------------- reading a member without the stray-state check

/// The check that a member holds no lockfile, `.sysand` or `output` of
/// its own can be skipped, and a caller can ask for everything else.
#[test]
fn a_member_with_a_lockfile_loads_when_stray_state_is_skipped() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/sysand-lock.toml", "");

    let workspace = fixture
        .load_with(MemberChecks::SkipStrayState)
        .expect("the member reads");
    assert_eq!(workspace.members().len(), 1);
}

/// The requirement that a member is a `model.toml` project is not what
/// was skipped.
#[test]
fn skipping_stray_state_still_refuses_a_json_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/.project.json", "{}");

    let error = fixture
        .load_with(MemberChecks::SkipStrayState)
        .expect_err("a json member is still refused");
    assert!(
        error.to_string().contains("must use the `model.toml`"),
        "{error}"
    );
}

/// Nor is the member's existence: a `members` entry naming nothing still
/// aborts the whole load.
#[test]
fn skipping_stray_state_still_refuses_a_missing_member() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"gone\""));
    fixture.project("app");

    let error = fixture
        .load_with(MemberChecks::SkipStrayState)
        .expect_err("`gone` is not there");
    assert!(error.to_string().contains("does not exist"), "{error}");
}

/// A member that inherits from the table *and* holds a lockfile of its
/// own.
///
/// Inheritance alone is `a_member_inherits_from_the_table`; the two
/// together are what a plain `LocalSrcProject` cannot read, since it
/// resolves with `None`.
#[test]
fn an_inherited_member_resolves_when_stray_state_is_skipped() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nversion = \"9.9.9\"\n",
    );
    fixture.write("app/model.toml", INHERITING);
    fixture.write("app/sysand-lock.toml", "");

    let workspace = fixture
        .load_with(MemberChecks::SkipStrayState)
        .expect("the member reads");
    assert_eq!(workspace.members()[0].project.version.to_string(), "9.9.9");
}

// ----------------------------------------------------------------------
// `project_at_path`: reading a project at a path a caller gave

use crate::project::ProjectRead as _;

/// The body of a member that inherits its metamodel from the default
/// group, so that reading it needs the table.
const INHERITS_METAMODEL: &str = "manifest-version = \"1.0\"\n\n[project]\n\
     publisher = \"acme\"\nname = \"member\"\nversion = \"1.0.0\"\nsources = []\n\
     metamodel.workspace = true\n";

/// The point of the helper: a listed member resolves.
#[test]
fn a_listed_member_is_read_with_the_table() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    fixture.write("app/model.toml", INHERITS_METAMODEL);

    let project = super::project_at_path(fixture.under("app"), None)
        .expect("a listed member is not an unlisted one");
    assert!(
        project.get_project().is_ok(),
        "a listed member resolves against its workspace's table"
    );
}

/// A project that merely *sits inside* a member is not one.
/// `WorkspaceTable::region` prefix-matches, so asking it alone hands the
/// table to `app/sub`; `region_of` adds the equality.
#[test]
fn a_project_nested_in_a_member_is_not_given_the_table() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    fixture.write("app/model.toml", INHERITS_METAMODEL);
    fixture.write("app/sub/model.toml", INHERITS_METAMODEL);

    let error = super::project_at_path(fixture.under("app/sub"), None)
        .expect_err("a project inside a member is not a member");
    assert_matches!(
        error,
        super::DiscoverError::Unlisted {
            project: _,
            root: _
        },
        "and it is under the root in neither list, which is row 25: {error:?}"
    );
}

/// One broken member does not stop another being read.
///
/// `Workspace::load_with` parses and validates every member, so a
/// sibling that does not parse would make the helper fall back to a
/// table-less read. Reading only the root manifest is what avoids that.
#[test]
fn a_broken_sibling_does_not_stop_a_member_being_read() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\", \"bad\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    fixture.write("app/model.toml", INHERITS_METAMODEL);
    fixture.write("bad/model.toml", "this is not a manifest\n");

    let project = super::project_at_path(fixture.under("app"), None)
        .expect("a sibling that does not parse is not this project's problem");
    assert!(
        project.get_project().is_ok(),
        "a sibling that does not parse is not this project's problem"
    );
}

/// A path outside any workspace is unchanged.
#[test]
fn a_project_outside_a_workspace_reads_table_less() {
    // `Fixture::at` puts the root in a subdirectory, so `outside/` is
    // genuinely above it.
    let fixture = Fixture::at(
        "ws",
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = []\n",
    );
    Fixture::write_at(
        &fixture.base().join("outside/model.toml"),
        "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"outside\"\n\
         version = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );

    let project = super::project_at_path(fixture.base().join("outside"), None)
        .expect("a project outside any workspace is not unlisted");
    assert!(
        project.get_project().is_ok(),
        "a plain project still reads: {:?}",
        project.get_project().err()
    );
}

/// A project under the root that is in neither list is refused, the
/// same answer walking up from inside it gives, so `sysand info --path`
/// on it fails as `cd` into it does.
#[test]
fn an_unlisted_project_at_a_path_is_refused() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    fixture.write("app/model.toml", INHERITS_METAMODEL);
    fixture.write("stray/model.toml", INHERITS_METAMODEL);

    let error = super::project_at_path(fixture.under("stray"), None)
        .expect_err("a project under the root in neither list is refused");
    assert_matches!(
        error,
        super::DiscoverError::Unlisted {
            project: _,
            root: _
        },
        "with row 25 and not some other failure: {error:?}"
    );
}

/// An excluded project is standalone, so it reads table-less.
#[test]
fn an_excluded_project_at_a_path_still_reads() {
    let fixture = Fixture::new(
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n\
         exclude = [\"vendor\"]\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    fixture.write("app/model.toml", INHERITS_METAMODEL);
    fixture.write(
        "vendor/dep/model.toml",
        "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"dep\"\n\
         version = \"1.0.0\"\nsources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );

    let project = super::project_at_path(fixture.under("vendor/dep"), None)
        .expect("an excluded project is standalone, not unlisted");
    assert!(
        project.get_project().is_ok(),
        "and it still reads: {:?}",
        project.get_project().err()
    );
}
