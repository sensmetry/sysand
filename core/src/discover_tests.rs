// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use camino::Utf8Path;
use camino_tempfile::tempdir;

use super::discover_project;

fn write(dir: &Utf8Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("write a manifest");
}

const PROJECT: &str = "\
manifest-version = \"1.0.0\"

[project]
publisher = \"acme\"
name = \"widgets\"
version = \"1.2.3\"
sources = []
";

const WORKSPACE: &str = "\
manifest-version = \"1.0.0\"

[workspace]
members = [\"app\"]
";

#[test]
fn a_model_toml_project_is_discovered() {
    let dir = tempdir().expect("a temporary directory");
    write(dir.path(), "model.toml", PROJECT);
    let found = discover_project(dir.path())
        .expect("readable")
        .expect("a project");
    assert_eq!(found.root_path(), dir.path());
}

/// A broken `model.toml` is still this directory's manifest: the walk
/// stops here so that reading it reports the problem, rather than
/// answering the parent's question.
#[test]
fn a_model_toml_that_is_not_toml_is_still_this_directorys_manifest() {
    let dir = tempdir().expect("a temporary directory");
    write(dir.path(), "model.toml", "this is not toml\n");
    let found = discover_project(dir.path())
        .expect("readable")
        .expect("a project");
    assert_eq!(found.root_path(), dir.path());
}

/// workspaces.md: a workspace root holds a `model.toml` that defines no
/// project. The walk goes past it.
#[test]
fn a_model_toml_workspace_is_walked_past() {
    let outer = tempdir().expect("a temporary directory");
    write(outer.path(), ".project.json", "{}");
    let root = outer.path().join("workspace");
    std::fs::create_dir(&root).expect("create the workspace directory");
    write(&root, "model.toml", WORKSPACE);

    let found = discover_project(&root)
        .expect("readable")
        .expect("a project");
    assert_eq!(found.root_path(), outer.path());
}

#[test]
fn a_json_project_is_still_discovered() {
    let dir = tempdir().expect("a temporary directory");
    write(dir.path(), ".project.json", "{}");
    let found = discover_project(dir.path())
        .expect("readable")
        .expect("a project");
    assert_eq!(found.root_path(), dir.path());
}
