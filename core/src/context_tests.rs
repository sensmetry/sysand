// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use super::ProjectContext;

// ------------------- the sources root

/// One lockfile for the whole workspace, at its root. Every `src_path` in it
/// is relative to that root, not to the member that ran `lock`.
#[cfg(feature = "filesystem")]
#[test]
fn the_model_workspace_root_wins_as_the_sources_root() {
    let dir = camino_tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = []\n",
    )
    .unwrap();
    let workspace = crate::workspace::model_toml::Workspace::load(dir.path()).unwrap();
    let root = workspace.root().to_owned();
    let ctx = ProjectContext {
        workspace: Some(workspace),
        current_project: Some(crate::project::local_src::LocalSrcProject::new_access(
            root.join("app"),
            None,
        )),
        ..Default::default()
    };
    assert_eq!(ctx.sources_root(), Some(root));
}

/// Outside a workspace it falls back to the project.
#[cfg(feature = "filesystem")]
#[test]
fn without_a_workspace_the_sources_root_is_the_project() {
    let ctx = ProjectContext {
        current_project: Some(crate::project::local_src::LocalSrcProject::new_access(
            "/proj", None,
        )),
        ..Default::default()
    };
    assert_eq!(
        ctx.sources_root().as_deref(),
        Some(camino::Utf8Path::new("/proj"))
    );
}

/// With neither there is no root.
#[cfg(feature = "filesystem")]
#[test]
fn with_neither_there_is_no_sources_root() {
    assert_eq!(ProjectContext::default().sources_root(), None);
}

// ------------------- writes through the context

/// A workspace of two members, `app` using `lib` through a `dir` usage when
/// `app_usage` is given, in a temporary directory.
#[cfg(feature = "filesystem")]
fn two_members(app_usage: &str) -> camino_tempfile::Utf8TempDir {
    let dir = camino_tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\", \"lib\"]\n",
    )
    .unwrap();
    for (name, usage) in [("app", app_usage), ("lib", "")] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
        std::fs::write(
            dir.path().join(name).join("model.toml"),
            format!(
                "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
                 version = \"1.0.0\"\nsources = []\n\
                 metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n{usage}"
            ),
        )
        .unwrap();
    }
    dir
}

#[cfg(feature = "filesystem")]
fn member_context(dir: &camino_tempfile::Utf8TempDir, member: &str) -> ProjectContext {
    use crate::{
        project::{ProjectRead as _, local_src::LocalSrcProject},
        workspace::model_toml::Workspace,
    };
    let workspace = Workspace::load(dir.path()).unwrap();
    let project = LocalSrcProject::new_member(
        workspace.root().join(member),
        None,
        workspace.table().clone(),
    );
    ProjectContext {
        current_definition: project.get_project().unwrap(),
        current_project: Some(project),
        workspace: Some(workspace),
        ..Default::default()
    }
}

/// An edit changes the manifest, the current definition and the member the
/// workspace holds alike.
#[cfg(feature = "filesystem")]
#[test]
fn edit_manifest_brings_the_snapshot_up_to_date() {
    use crate::project::ProjectDefinition;
    let dir = two_members("");
    let mut ctx = member_context(&dir, "app");

    ctx.edit_manifest(|document| {
        crate::manifest::edit::set_project_key(
            document,
            crate::manifest::edit::ProjectKey::Version,
            "2.0.0".into(),
        )
    })
    .unwrap();

    let Some(ProjectDefinition::Toml(definition)) = &ctx.current_definition else {
        panic!("a `model.toml` definition");
    };
    assert_eq!(definition.version.to_string(), "2.0.0");
    let member = &ctx.workspace.as_ref().unwrap().members()[0];
    assert_eq!(member.project.version.to_string(), "2.0.0");
    assert!(
        std::fs::read_to_string(dir.path().join("app/model.toml"))
            .unwrap()
            .contains("2.0.0")
    );
}

/// An edit that makes the members cyclic is written, and refused when the
/// cross-member checks run again over the parsed members.
#[cfg(feature = "filesystem")]
#[test]
fn edit_project_rechecks_the_members() {
    use crate::context::{EditError, ReloadError};
    let dir =
        two_members("[[inline-usages]]\ndir = \"../lib\"\npublisher = \"acme\"\nname = \"lib\"\n");
    let mut ctx = member_context(&dir, "lib");

    let result = ctx.edit_manifest(|document| {
        crate::manifest::edit::push_usage(
            document,
            &crate::manifest::usage::MemberUsage::Declared(
                crate::manifest::usage::Usage::SourcePath {
                    dir: "../app".into(),
                    publisher: crate::model::IndexPublisher::parse("acme").unwrap(),
                    name: crate::model::IndexName::parse("app").unwrap(),
                },
            ),
        )
    });
    assert!(matches!(
        result,
        Err(EditError::Reload(ReloadError::Workspace(_)))
    ));
}

/// Without a current project there is nothing to edit.
#[cfg(feature = "filesystem")]
#[test]
fn edit_project_needs_a_current_project() {
    use crate::context::EditError;
    let mut ctx = ProjectContext::default();
    assert!(matches!(
        ctx.edit_project(|_| Ok::<(), std::convert::Infallible>(())),
        Err(EditError::NoProject)
    ));
}
