// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensember.com>

use camino::Utf8PathBuf;
use camino_tempfile::Utf8TempDir;

use super::{Classification, LoadError, LoadFailure, Workspace};
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
}

impl Fixture {
    /// `root` is the body of the root `model.toml`.
    fn new(root: &str) -> Self {
        let fixture = Self {
            dir: camino_tempfile::tempdir().expect("a temporary directory"),
        };
        fixture.write("model.toml", root);
        fixture
    }

    /// The root, canonicalized, which is what [`Workspace::load`]
    /// reports and what [`Workspace::classify`] relativizes against.
    fn root(&self) -> Utf8PathBuf {
        self.dir
            .path()
            .canonicalize_utf8()
            .expect("the temporary directory resolves")
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.dir.path().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the parent directories");
        }
        std::fs::write(path, content).expect("write a file");
    }

    fn mkdir(&self, path: &str) {
        std::fs::create_dir_all(self.dir.path().join(path)).expect("create a directory");
    }

    /// A member directory holding a valid `model.toml` project.
    fn project(&self, at: &str) {
        self.write(&format!("{at}/model.toml"), PROJECT);
    }

    fn load(&self) -> Result<Workspace, LoadError> {
        Workspace::load(self.dir.path())
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
        ProjectDeserializationError::Invalid { report, .. } => report.diagnostics(),
        other @ ProjectDeserializationError::Syntax { .. } => {
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

fn classified() -> (Utf8TempDir, Workspace) {
    let fixture = Fixture::new(
        "manifest-version = 1\n\n[workspace]\n\
         members = [\"app\", \"vendor/kept\"]\n\
         exclude = [\"vendor\", \"app\"]\n",
    );
    fixture.project("app");
    fixture.project("vendor/kept");
    let workspace = fixture.loaded();
    (fixture.dir, workspace)
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
    let (_dir, workspace) = classified();
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
