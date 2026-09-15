// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensember.com>

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;

use super::{
    Classification, DiscoverError, LoadError, LoadFailure, MemberChecks, SelectError, Selection,
    Selector, Workspace, candidate_root, confirm_root,
};
use crate::manifest::diagnostics::Diagnostic;
use crate::project::utils::ProjectDeserializationError;

const PROJECT: &str = "\
manifest-version = 1

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
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
        self.dir
            .path()
            .canonicalize_utf8()
            .expect("the temporary directory resolves")
    }

    /// The root, canonicalized, which is what [`Workspace::load`]
    /// reports and what [`Workspace::classify`] relativizes against.
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

    /// The same, with a chosen project name. Every member written by
    /// [`Fixture::project`] is named `widgets`, which would make half of
    /// the selection tests pass or fail by accident.
    fn project_named(&self, at: &str, name: &str) {
        self.write(
            &format!("{at}/model.toml"),
            &PROJECT.replace("name = \"widgets\"", &format!("name = \"{name}\"")),
        );
    }

    fn load(&self) -> Result<Workspace, LoadError> {
        Workspace::load(self.dir.path().join(self.at))
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

/// The diagnostics raised against the *root* manifest.
fn root_diagnostics(failure: &LoadFailure) -> &[Diagnostic] {
    match failure.root.as_ref().expect("root diagnostics") {
        ProjectDeserializationError::Invalid { report, path: _ } => report.diagnostics(),
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
    format!("manifest-version = 1\n\n[workspace]\nmembers = [{members}]\n")
}

// ----------------------------------------------------------- loading

#[test]
fn a_workspace_loads_its_members() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"lib\""));
    fixture.project("app");
    fixture.project("lib");

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
    let fixture = Fixture::new("manifest-version = 1\n\n[workspace]\n");
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

/// The file that is *there* is named. A member holding only `.meta.json`
/// told to look at `.project.json` would send the reader after a file
/// that does not exist.
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

/// workspaces.md: "A member cannot be such a root in any case, since the
/// two tables never share a file and a member must define a project" --
/// an assertion, which this is the enforcement of. `detect_format`
/// cannot see it: the directory holds a `model.toml` and nothing else.
#[test]
fn a_member_that_is_a_workspace_root_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/model.toml", "manifest-version = 1\n\n[workspace]\n");
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
    // The clause is written out in this module rather than stringified
    // from the error that owns it; this holds the two together.
    let owned = crate::project::local_src::LocalSrcError::AmbiguousFormat {
        dir: Utf8PathBuf::from("app"),
        json: ".project.json",
    }
    .to_string();
    // The owned message wraps and indents its second clause; compare
    // the words, which is what has to stay in step.
    let words = |text: &str| {
        text.split_once(';')
            .expect("the ambiguity message has two clauses")
            .1
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(words(&message), words(&owned));
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

/// workspaces.md: "Standalone projects under an `exclude`d directory
/// keep their state legitimately; the check applies to members only."
#[test]
fn an_excluded_project_keeps_its_own_state() {
    // The state sits on the `exclude` entry itself, so that "run the
    // member checks over `exclude` too" is a mutation this test catches.
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
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
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n\
         exclude = [\"vendor\", \"gone\"]\n",
    );
    fixture.project("app");
    fixture.mkdir("vendor");

    assert_eq!(fixture.loaded().members().len(), 1);
}

#[test]
fn a_member_diagnostic_points_at_the_entry_that_named_it() {
    let fixture = Fixture::new("manifest-version = 1\n\n[workspace]\nmembers = [\n  \"app\",\n]\n");
    let failure = fixture.failure();
    let position = root_diagnostics(&failure)[0]
        .position
        .as_ref()
        .expect("the diagnostic points at the entry");
    assert_eq!(position.file, "model.toml");
    assert_eq!(position.line, 5);
}

/// A path is where it points and how it was spelled; where it was
/// *written* is provenance for a message.
#[test]
fn a_member_position_does_not_change_path_equality() {
    use crate::manifest::diagnostics::Position;
    use crate::manifest::workspace_table::MemberPath;

    let path = crate::utils::normalize_relative("app").expect("a valid path");
    let without = MemberPath {
        raw: "app".to_owned(),
        path: path.clone(),
        at: None,
    };
    let with = MemberPath {
        raw: "app".to_owned(),
        path,
        at: Some(Position::at("model.toml", 5, 3)),
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
    match Workspace::load(dir.path()) {
        Err(LoadError::Io(_)) => {}
        other => panic!("expected an IO error, got `{other:?}`"),
    }
}

// ------------------------------------ inheritance and classification

const INHERITING: &str = "\
manifest-version = 1

[project]
publisher = \"acme\"
name = \"widgets\"
version.workspace = true
sources = []
";

/// A member's own marker cannot be followed by anything that does not
/// know which workspace the member belongs to, so the load resolves it
/// against the table.
#[test]
fn a_member_inherits_from_the_table() {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nversion = \"9.9.9\"\n",
    );
    fixture.write("app/model.toml", INHERITING);

    let workspace = fixture.loaded();
    assert_eq!(workspace.members()[0].project.version.to_string(), "9.9.9");
}

#[test]
fn a_member_whose_own_manifest_is_broken_is_named_by_its_own_file() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.write("app/model.toml", "manifest-version = 1\n\n[project]\n");

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
        "manifest-version = 1\n\n[workspace]\n\
         members = [\"app\", \"vendor/kept\"]\n\
         exclude = [\"vendor\", \"app\"]\n",
    );
    fixture.project("app");
    fixture.project("vendor/kept");
    let workspace = fixture.loaded();
    (fixture, workspace)
}

fn at(workspace: &Workspace, path: &str) -> Utf8PathBuf {
    if path.is_empty() {
        workspace.root().to_owned()
    } else {
        workspace.root().join(path)
    }
}

#[test]
fn a_listed_directory_is_a_member() {
    let (_fixture, workspace) = classified();
    match workspace
        .classify(&at(&workspace, "app"))
        .expect("classifies")
    {
        Classification::Member(member) => assert_eq!(member.path.raw, "app"),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

#[test]
fn an_excluded_directory_is_excluded() {
    let (_fixture, workspace) = classified();
    match workspace
        .classify(&at(&workspace, "vendor"))
        .expect("classifies")
    {
        Classification::Excluded(path) => assert_eq!(path.raw, "vendor"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

/// workspaces.md: "Anything under one of them is outside the workspace."
#[test]
fn a_directory_under_an_excluded_one_is_excluded() {
    let (_fixture, workspace) = classified();
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "vendor/other/deep"))
            .expect("classifies"),
        Classification::Excluded(_)
    ));
}

/// A tie goes to `members`: nothing refuses a path written in both
/// lists, so the precedence has to be decided here.
#[test]
fn a_directory_in_both_lists_is_a_member() {
    let (_fixture, workspace) = classified();
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "app"))
            .expect("classifies"),
        Classification::Member(_)
    ));
}

/// The longest match wins, so a member listed *inside* an excluded
/// directory is still a member -- and so is everything under it.
#[test]
fn a_member_under_an_excluded_directory_wins() {
    let (_fixture, workspace) = classified();
    match workspace
        .classify(&at(&workspace, "vendor/kept/src"))
        .expect("classifies")
    {
        Classification::Member(member) => assert_eq!(member.path.raw, "vendor/kept"),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

/// workspaces.md: under an excluded directory "a workspace root there is
/// a legitimate, separate workspace". The nesting check rests on this.
#[test]
fn a_workspace_root_under_an_excluded_directory_is_excluded() {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.write(
        "vendor/inner/model.toml",
        "manifest-version = 1\n\n[workspace]\n",
    );
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "vendor/inner"))
            .expect("classifies"),
        Classification::Excluded(_)
    ));
}

/// `exclude` entries may nest -- the cross-entry checks are off for
/// that list -- so the most specific one is the one that answers.
#[test]
fn the_most_specific_excluded_entry_answers() {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n\
         exclude = [\"vendor\", \"vendor/deep\"]\n",
    );
    fixture.project("app");
    let workspace = fixture.loaded();
    match workspace
        .classify(&at(&workspace, "vendor/deep/inner"))
        .expect("classifies")
    {
        Classification::Excluded(path) => assert_eq!(path.raw, "vendor/deep"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

/// The specificity rule runs both ways: an `exclude` entry *inside* a
/// member carves that subtree back out. Nothing refuses the pair, so
/// something has to decide it.
#[test]
fn an_excluded_directory_inside_a_member_is_excluded() {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"app/gen\"]\n",
    );
    fixture.project("app");
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "app/src"))
            .expect("classifies"),
        Classification::Member(_)
    ));
    match workspace
        .classify(&at(&workspace, "app/gen/out"))
        .expect("classifies")
    {
        Classification::Excluded(path) => assert_eq!(path.raw, "app/gen"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
}

#[test]
fn anything_else_is_unlisted() {
    let (_fixture, workspace) = classified();
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "elsewhere"))
            .expect("classifies"),
        Classification::Unlisted
    ));
    assert!(matches!(
        workspace.classify(&at(&workspace, "")).expect("classifies"),
        Classification::Unlisted
    ));
}

#[test]
fn a_directory_outside_the_root_is_outside() {
    let (_fixture, workspace) = classified();
    let outside = workspace
        .root()
        .parent()
        .expect("the root has a parent")
        .to_owned();
    assert!(matches!(
        workspace.classify(&outside).expect("classifies"),
        Classification::Outside
    ));
}

/// `app2` starts with `app`, but is not under it.
#[test]
fn classification_is_by_whole_components() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let workspace = fixture.loaded();

    assert!(matches!(
        workspace
            .classify(&at(&workspace, "app"))
            .expect("classifies"),
        Classification::Member(_)
    ));
    assert!(matches!(
        workspace
            .classify(&at(&workspace, "app2"))
            .expect("classifies"),
        Classification::Unlisted
    ));
}

// --------------------------------------------------- the upward walk

const EMPTY_WORKSPACE: &str = "manifest-version = 1\n\n[workspace]\n";

#[test]
fn a_workspace_is_found_in_the_starting_directory() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    assert_eq!(
        candidate_root(fixture.root()).expect("readable"),
        Some(fixture.root())
    );
}

#[test]
fn a_workspace_is_found_in_an_ancestor() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.mkdir("app/src/deep");
    assert_eq!(
        candidate_root(fixture.under("app/src/deep")).expect("readable"),
        Some(fixture.root())
    );
}

/// Not hermetic, and cannot be: the walk runs to the filesystem root.
/// A `model.toml` in an ancestor of the temporary directory would fail
/// this.
#[test]
fn no_workspace_above_the_starting_directory_is_none() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    assert_eq!(candidate_root(dir.path()).expect("readable"), None);
}

/// A member's directory holds a `[project]`, which is not a root.
#[test]
fn the_walk_goes_past_a_project() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    assert_eq!(
        candidate_root(fixture.under("app")).expect("readable"),
        Some(fixture.root())
    );
}

/// `peek` answers `None`. `is_project_file` calls the same directory a
/// project, which is right for its question and not for this one.
#[test]
fn the_walk_goes_past_a_manifest_with_neither_table() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    fixture.write("odd/model.toml", "manifest-version = 1\n");
    assert_eq!(
        candidate_root(fixture.under("odd")).expect("readable"),
        Some(fixture.root())
    );
}

/// Walking past would answer *no workspace* for a directory whose
/// manifest may well be one.
#[test]
fn a_manifest_the_walk_cannot_peek_is_reported() {
    let fixture = Fixture::new(EMPTY_WORKSPACE);
    fixture.write("broken/model.toml", "this is not toml\n");
    match candidate_root(fixture.under("broken")) {
        Err(DiscoverError::Manifest(error)) => {
            let text = error.to_string();
            assert!(text.contains("broken/model.toml"), "{text}");
        }
        other => panic!("expected a manifest error, got `{other:?}`"),
    }
}

/// The walk climbs with `Utf8Path::parent`, which is textual. Without
/// canonicalizing first, a path that detours through another directory
/// finds the right workspace by the wrong name, and hands back a root
/// spelled with a `..` in it.
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
        candidate_root(roundabout).expect("readable"),
        Some(fixture.root())
    );
}

#[test]
fn a_nested_workspace_is_refused() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);

    match confirm_root(fixture.root()) {
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
        "manifest-version = 1\n\n[workspace]\nexclude = [\"inner\"]\n",
    );
    confirm_root(fixture.root()).expect("the outer excludes it");
}

/// "the outer workspace lists the candidate's directory, **or a
/// directory above it**, in `exclude`".
#[test]
fn a_nested_workspace_excluded_by_a_directory_above_it_is_confirmed() {
    let fixture = Fixture::at("vendor/inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = 1\n\n[workspace]\nexclude = [\"vendor\"]\n",
    );
    confirm_root(fixture.root()).expect("an excluded directory above it");
}

/// "whether the outer workspace is itself nested is checked when a
/// command runs inside it". The first outer settles the question either
/// way; a third workspace above it is not this command's problem.
#[test]
fn the_outer_workspace_is_not_itself_checked_for_nesting() {
    let fixture = Fixture::at("b/c", EMPTY_WORKSPACE);
    fixture.above(
        "b/model.toml",
        "manifest-version = 1\n\n[workspace]\nexclude = [\"c\"]\n",
    );
    fixture.above("model.toml", EMPTY_WORKSPACE);
    confirm_root(fixture.root()).expect("`b` excludes `c`; `a` is `b`'s problem");
}

#[test]
fn an_outer_workspace_far_above_is_still_found() {
    let fixture = Fixture::at("a/b/c/inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);
    assert!(matches!(
        confirm_root(fixture.root()),
        Err(DiscoverError::Nested { inner: _, outer: _ })
    ));
}

/// `parse` runs the table's own validation, and its report is
/// all-or-nothing: a table whose `exclude` does not parse cannot answer
/// the question being asked of it.
#[test]
fn an_outer_whose_member_paths_do_not_parse_is_reported() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = 1\n\n[workspace]\nmembers = [\"/absolute\"]\n",
    );
    assert!(matches!(
        confirm_root(fixture.root()),
        Err(DiscoverError::Manifest(_))
    ));
}

/// Parsing is enough to answer the exclusion question; loading is not.
/// A command inside a separate workspace must not fail because a
/// neighbour's member directory is missing.
#[test]
fn an_outer_whose_member_is_broken_still_confirms_the_candidate() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above(
        "model.toml",
        "manifest-version = 1\n\n[workspace]\nmembers = [\"gone\"]\nexclude = [\"inner\"]\n",
    );
    confirm_root(fixture.root()).expect("the outer's own members are not this command's problem");
}

// --------------------------------------------------------- discovery

fn discover_error(from: impl AsRef<Utf8Path>) -> DiscoverError {
    Workspace::discover(from).expect_err("expected discovery to fail")
}

#[test]
fn discover_loads_the_workspace_it_found() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("app/src");

    let workspace = Workspace::discover(fixture.under("app/src"))
        .expect("readable")
        .expect("a workspace");
    assert_eq!(workspace.root(), fixture.root());
    assert_eq!(workspace.members().len(), 1);
}

#[test]
fn discover_reports_a_load_failure_from_the_candidate() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    assert!(matches!(
        discover_error(fixture.root()),
        DiscoverError::Load(_)
    ));
}

/// `peek` answers `Workspace` for a file carrying both tables, so such a
/// member becomes a candidate. Ruling on nesting first would report the
/// wrong fault.
#[test]
fn a_candidate_defining_both_tables_fails_as_that_not_as_nesting() {
    let fixture = Fixture::at("inner", &format!("{EMPTY_WORKSPACE}\n{PROJECT}"));
    fixture.above("model.toml", EMPTY_WORKSPACE);

    match discover_error(fixture.root()) {
        DiscoverError::Load(_) => {}
        other => panic!("expected the candidate's own manifest error, got `{other:?}`"),
    }
}

#[test]
fn a_member_directory_starts_in_that_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let workspace = fixture.loaded();
    match workspace.start(fixture.under("app")).expect("classifies") {
        Selection::Member(member) => assert_eq!(member.path.raw, "app"),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

#[test]
fn a_directory_under_a_member_starts_in_that_member() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("app/src/deep");
    let workspace = fixture.loaded();
    match workspace
        .start(fixture.under("app/src/deep"))
        .expect("classifies")
    {
        Selection::Member(member) => assert_eq!(member.path.raw, "app"),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

/// "This is the only kind of non-member project that may sit under a
/// workspace root."
#[test]
fn an_excluded_project_starts_standalone() {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"vendor\"]\n",
    );
    fixture.project("app");
    fixture.project("vendor/other");
    let workspace = fixture.loaded();
    match workspace
        .start(fixture.under("vendor/other"))
        .expect("classifies")
    {
        Selection::Standalone(path) => assert_eq!(path, fixture.under("vendor/other")),
        other => panic!("expected standalone, got `{other:?}`"),
    }
}

#[test]
fn an_unlisted_project_is_refused() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.project("stray");
    let workspace = fixture.loaded();
    let error = workspace
        .start(fixture.under("stray"))
        .expect_err("an unlisted project");
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
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.start(fixture.under("notes")).expect("classifies"),
        Selection::Workspace
    ));
}

/// "the workspace root is never a member, so commands run from the
/// workspace root always target the whole workspace".
#[test]
fn the_root_starts_in_the_workspace() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.start(fixture.root()).expect("classifies"),
        Selection::Workspace
    ));
}

/// A project above the root is a different project, which this
/// workspace has no opinion about.
#[test]
fn a_project_above_the_root_is_not_the_starting_project() {
    let fixture = Fixture::at("ws", &workspace_with("\"app\""));
    fixture.project("app");
    fixture.mkdir("notes");
    fixture.above("model.toml", PROJECT);
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.start(fixture.under("notes")).expect("classifies"),
        Selection::Workspace
    ));
}

#[test]
fn a_directory_outside_the_workspace_is_refused() {
    let fixture = Fixture::at("ws", &workspace_with("\"app\""));
    fixture.project("app");
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.start(fixture.base()),
        Err(DiscoverError::NotUnderRoot { dir: _, root: _ })
    ));
}

/// `relativize_path` asserts absolute, dot-free paths with a plain
/// `assert!`, which panics in release too.
///
/// A `..` component and not a `.` one: `Utf8Path::components` drops `.`
/// on its way past, so a path spelled with one is already canonical by
/// the time anything looks, and a test built on it would pin nothing.
#[test]
fn a_starting_path_with_a_parent_component_is_accepted() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    let roundabout = fixture.under("app").join("..").join("app");

    let workspace = Workspace::discover(&roundabout)
        .expect("readable")
        .expect("a workspace");
    assert_eq!(workspace.root(), fixture.root());
    assert!(matches!(
        workspace.start(&roundabout).expect("classifies"),
        Selection::Member(_)
    ));
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

/// "Literally" is read as the spelling the author wrote, which is what
/// `MemberPath::raw` is kept for: another spelling of the same directory
/// selects nothing.
#[test]
fn a_member_path_is_matched_as_written() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project_named("app", "alpha");
    let workspace = fixture.loaded();

    assert_eq!(workspace.member("app").expect("a member").path.raw, "app");
    assert!(matches!(
        workspace.member("./app/"),
        Err(SelectError::NoSuchMember {
            root: _,
            selector: _,
            known: _
        })
    ));
}

#[test]
fn a_member_is_selected_by_its_project_name() {
    let workspace = two_members().loaded();
    assert_eq!(workspace.member("beta").expect("a member").path.raw, "lib");
}

/// The case that makes the rule a union rather than an order: guessing
/// would silently act on the wrong project.
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
    fixture.project_named("app", "alpha");
    fixture.project_named("lib", "alpha");
    let workspace = fixture.loaded();

    assert!(matches!(
        workspace.member("alpha"),
        Err(SelectError::AmbiguousMember {
            selector: _,
            matches: _
        })
    ));
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

// --------------------------------------------------------------- scope

fn member_of<'a>(selection: &Selection<'a>) -> &'a str {
    match selection {
        Selection::Member(member) => member.path.raw.as_str(),
        other => panic!("expected a member, got `{other:?}`"),
    }
}

/// A workspace with a member, an excluded standalone project, a
/// non-member directory and an unlisted project.
fn scoped() -> Fixture {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\", \"lib\"]\n\
         exclude = [\"vendor\"]\n",
    );
    fixture.project_named("app", "alpha");
    fixture.project_named("lib", "beta");
    fixture.project_named("vendor/thing", "gamma");
    fixture.mkdir("notes");
    fixture.project_named("stray", "delta");
    fixture
}

#[test]
fn the_root_selects_the_whole_workspace() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .select(fixture.root(), Selector::Current)
            .expect("selects"),
        Selection::Workspace
    ));
}

#[test]
fn a_non_member_directory_selects_the_whole_workspace() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .select(fixture.under("notes"), Selector::Current)
            .expect("selects"),
        Selection::Workspace
    ));
}

#[test]
fn a_member_directory_selects_that_member() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select(fixture.under("app"), Selector::Current)
        .expect("selects");
    assert_eq!(member_of(&selection), "app");
}

#[test]
fn a_directory_under_a_member_selects_that_member() {
    let fixture = scoped();
    fixture.mkdir("app/src/deep");
    let workspace = fixture.loaded();
    let selection = workspace
        .select(fixture.under("app/src/deep"), Selector::Current)
        .expect("selects");
    assert_eq!(member_of(&selection), "app");
}

#[test]
fn the_workspace_flag_forces_the_whole_workspace_from_a_member() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .select(fixture.under("app"), Selector::Workspace)
            .expect("selects"),
        Selection::Workspace
    ));
}

#[test]
fn the_member_flag_picks_one_from_the_root() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select(fixture.root(), Selector::Member("lib"))
        .expect("selects");
    assert_eq!(member_of(&selection), "lib");
}

/// Position is only the default.
#[test]
fn the_member_flag_picks_another_from_inside_a_member() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select(fixture.under("app"), Selector::Member("lib"))
        .expect("selects");
    assert_eq!(member_of(&selection), "lib");
}

#[test]
fn an_excluded_project_selects_standalone() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    match workspace
        .select(fixture.under("vendor/thing"), Selector::Current)
        .expect("selects")
    {
        Selection::Standalone(dir) => assert_eq!(dir, fixture.under("vendor/thing")),
        other => panic!("expected standalone, got `{other:?}`"),
    }
}

#[test]
fn a_selector_beats_standing_in_an_excluded_project() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select(fixture.under("vendor/thing"), Selector::Member("app"))
        .expect("selects");
    assert_eq!(member_of(&selection), "app");
}

#[test]
fn the_workspace_flag_beats_standing_in_an_excluded_project() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .select(fixture.under("vendor/thing"), Selector::Workspace)
            .expect("selects"),
        Selection::Workspace
    ));
}

#[test]
fn an_unlisted_project_is_refused_by_select() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.select(fixture.under("stray"), Selector::Current),
        Err(DiscoverError::Unlisted {
            project: _,
            root: _
        })
    ));
}

/// "a project directory under the root that is neither a member nor
/// excluded is the hard error of Included projects, whatever the
/// command" -- a selector does not excuse standing in one.
#[test]
fn an_unlisted_project_is_refused_by_select_with_a_selector() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.select(fixture.under("stray"), Selector::Member("app")),
        Err(DiscoverError::Unlisted {
            project: _,
            root: _
        })
    ));
}

#[test]
fn select_member_from_a_member_directory_needs_no_flag() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select_member(fixture.under("app"), None)
        .expect("selects");
    assert_eq!(member_of(&selection), "app");
}

#[test]
fn select_member_from_the_root_is_refused() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    match workspace.select_member(fixture.root(), None) {
        Err(error @ DiscoverError::NeedsProject) => assert_eq!(
            error.to_string(),
            "this command needs a project; run it from a member directory or pass \
             `--member <path-or-name>`"
        ),
        other => panic!("expected `NeedsProject`, got `{other:?}`"),
    }
}

#[test]
fn select_member_from_a_non_member_directory_is_refused() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace.select_member(fixture.under("notes"), None),
        Err(DiscoverError::NeedsProject)
    ));
}

#[test]
fn select_member_with_the_flag_from_the_root_is_accepted() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    let selection = workspace
        .select_member(fixture.root(), Some("lib"))
        .expect("selects");
    assert_eq!(member_of(&selection), "lib");
}

/// A writing command in an excluded standalone project acts on that
/// project, which is why `select_member` cannot return a `&Member`.
#[test]
fn select_member_in_an_excluded_project_is_standalone() {
    let fixture = scoped();
    let workspace = fixture.loaded();
    assert!(matches!(
        workspace
            .select_member(fixture.under("vendor/thing"), None)
            .expect("selects"),
        Selection::Standalone(_)
    ));
}

// ------------------------------------------------------ table regions

use crate::manifest::workspace_table::{Region, WorkspaceTable};
use typed_path::Utf8UnixPath;

/// A table with all four shapes: a plain member, a plain exclusion, a
/// member inside an exclusion, an exclusion inside a member, and a path
/// written in both lists.
const SHAPES: &str = "\
manifest-version = 1

[workspace]
members = [\"app\", \"vendor/kept\", \"both\"]
exclude = [\"vendor\", \"app/gen\", \"both\", \"deep/a/b\"]
";

fn shapes() -> WorkspaceTable {
    let fixture = Fixture::new(SHAPES);
    fixture.project_named("app", "alpha");
    fixture.project_named("vendor/kept", "beta");
    fixture.project_named("both", "gamma");
    let (_root, table) = super::discover_table(fixture.root())
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
    assert!(matches!(region_of(&table, "notes"), Region::Unlisted));
    assert!(matches!(region_of(&table, "deep/a"), Region::Unlisted));
}

/// The three combinations nothing refuses: a path in both lists, an
/// `exclude` inside a member, and a member inside an `exclude`.
#[test]
fn a_region_is_decided_by_the_most_specific_entry() {
    let table = shapes();

    // An exact tie goes to `members`.
    assert!(matches!(region_of(&table, "both"), Region::Member(2, _)));
    // An `exclude` inside a member carves that subtree back out.
    match region_of(&table, "app/gen/out") {
        Region::Excluded(entry) => assert_eq!(entry.raw, "app/gen"),
        other => panic!("expected excluded, got `{other:?}`"),
    }
    // A member inside an `exclude` is still a member.
    match region_of(&table, "vendor/kept/src") {
        Region::Member(index, entry) => {
            assert_eq!(index, 1);
            assert_eq!(entry.raw, "vendor/kept");
        }
        other => panic!("expected a member, got `{other:?}`"),
    }
}

/// Not a proof that `classify` delegates -- a faithful duplicate of the
/// rule would pass -- but what fails if a second rule ever diverges.
#[test]
fn classify_agrees_with_region_everywhere() {
    let fixture = Fixture::new(SHAPES);
    fixture.project_named("app", "alpha");
    fixture.project_named("vendor/kept", "beta");
    fixture.project_named("both", "gamma");
    let workspace = fixture.loaded();

    for path in [
        "app",
        "app/src",
        "app/gen",
        "app/gen/out",
        "vendor",
        "vendor/other",
        "vendor/kept",
        "vendor/kept/src",
        "both",
        "both/inner",
        "deep",
        "deep/a",
        "deep/a/b",
        "deep/a/b/c",
        "notes",
    ] {
        let region = workspace.table().region(Utf8UnixPath::new(path));
        let classification = workspace
            .classify(&fixture.under(path))
            .expect("classifies");
        let agrees = match (region, classification) {
            (Region::Member(_, entry), Classification::Member(member)) => {
                entry.raw == member.path.raw
            }
            (Region::Excluded(a), Classification::Excluded(b)) => a.raw == b.raw,
            (Region::Unlisted, Classification::Unlisted) => true,
            _ => false,
        };
        assert!(agrees, "`{path}`: {region:?} vs the loaded classification");
    }
}

/// Creating a new project cannot be blocked by an unrelated member's
/// mistake.
#[test]
fn a_table_is_discovered_without_loading_its_members() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"gone\""));
    fixture.project_named("app", "alpha");

    let (root, table) = super::discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(root, fixture.root());
    assert_eq!(table.members.len(), 2);

    assert!(
        Workspace::discover(fixture.root()).is_err(),
        "loading the same workspace must still refuse it"
    );
}

#[test]
fn a_discovered_table_is_still_checked_for_nesting() {
    let fixture = Fixture::at("inner", EMPTY_WORKSPACE);
    fixture.above("model.toml", EMPTY_WORKSPACE);
    assert!(matches!(
        super::discover_table(fixture.root()),
        Err(DiscoverError::Nested { inner: _, outer: _ })
    ));
}

/// The candidate is parsed before the nesting verdict, so a file
/// carrying both tables fails as that.
#[test]
fn a_candidate_defining_both_tables_still_fails_as_that() {
    let fixture = Fixture::at("inner", &format!("{EMPTY_WORKSPACE}\n{PROJECT}"));
    fixture.above("model.toml", EMPTY_WORKSPACE);
    match super::discover_table(fixture.root()) {
        Err(DiscoverError::Manifest(_)) => {}
        other => panic!("expected the candidate's own manifest error, got `{other:?}`"),
    }
}

// ------------------------------- `start_region` reads no member

/// The property the CLI depends on: the walk answers over the table
/// alone, so a member that `Workspace::load` refuses does not stop it.
///
/// `workspaces.md` "Stray per-member state" makes a member's own
/// `sysand-lock.toml` an error, and the walk must still answer for a
/// member that has one.
#[test]
fn start_region_ignores_a_members_own_lockfile() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/sysand-lock.toml", "");
    fixture
        .load()
        .expect_err("the tree is one `load` refuses, which is the point");

    let (root, table) = super::discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(
        super::start_region(&root, &table, fixture.under("app")).expect("classifies"),
        super::StartRegion::Member(0)
    );
}

/// And the same for an entry naming nothing: only the entry the walk
/// matched is consulted.
#[test]
fn start_region_ignores_a_member_that_does_not_exist() {
    let fixture = Fixture::new(&workspace_with("\"app\", \"gone\""));
    fixture.project("app");
    fixture.load().expect_err("`load` refuses a missing member");

    let (root, table) = super::discover_table(fixture.root())
        .expect("the table reads")
        .expect("a workspace");
    assert_eq!(
        super::start_region(&root, &table, fixture.under("app")).expect("classifies"),
        super::StartRegion::Member(0)
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

/// Nor is the member's existence -- and a `members` entry naming
/// nothing still aborts the whole load.
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
        "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\nversion = \"9.9.9\"\n",
    );
    fixture.write("app/model.toml", INHERITING);
    fixture.write("app/sysand-lock.toml", "");

    let workspace = fixture
        .load_with(MemberChecks::SkipStrayState)
        .expect("the member reads");
    assert_eq!(workspace.members()[0].project.version.to_string(), "9.9.9");
}

/// Through the upward walk, not only through `load`.
#[test]
fn discover_with_skips_stray_state_too() {
    let fixture = Fixture::new(&workspace_with("\"app\""));
    fixture.project("app");
    fixture.write("app/sysand-lock.toml", "");

    let workspace = Workspace::discover_with(fixture.under("app"), MemberChecks::SkipStrayState)
        .expect("discovery succeeds")
        .expect("a workspace");
    assert_eq!(workspace.members().len(), 1);
}
