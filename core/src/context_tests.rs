// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

#[cfg(feature = "filesystem")]
use super::ProjectContext;
#[cfg(feature = "filesystem")]
use crate::{
    manifest::{edit, usage},
    project::local_src,
    workspace::model_toml::Workspace,
};

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
    let workspace = Workspace::load(dir.path()).unwrap();
    let root = workspace.root().to_owned();
    let ctx = ProjectContext {
        workspace: Some(workspace),
        current_project: Some(local_src::LocalSrcProject::new_access(
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
        current_project: Some(local_src::LocalSrcProject::new_access("/proj", None)),
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
        r#"manifest-version = "1.0"

[workspace]
members = ["app", "lib"]
"#,
    )
    .unwrap();
    for (name, usage) in [("app", app_usage), ("lib", "")] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
        std::fs::write(
            dir.path().join(name).join("model.toml"),
            format!(
                r#"manifest-version = "1.0"

[project]
publisher = "acme"
name = "{name}"
version = "1.0.0"
sources = []
metamodel = "https://www.omg.org/spec/SysML/20250201"
{usage}"#
            ),
        )
        .unwrap();
    }
    dir
}

#[cfg(feature = "filesystem")]
fn member_context(dir: &camino_tempfile::Utf8TempDir, member: &str) -> ProjectContext {
    use crate::project::{ProjectRead as _, local_src::LocalSrcProject};
    let workspace = Workspace::load(dir.path()).unwrap();
    let dir = workspace.root().join(member);
    let manifest = workspace.member_at(&dir).unwrap().unwrap().project.clone();
    let project = LocalSrcProject::new_member(dir, None, manifest);
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
        edit::set_project_key(document, edit::ProjectKey::Version, "2.0.0".into())
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

/// An edit that makes the members cyclic is refused before it is written.
#[cfg(feature = "filesystem")]
#[test]
fn edit_manifest_rechecks_the_members_before_writing() {
    use crate::context::{EditError, ManifestStep};
    let dir = two_members(
        r#"[[inline-usages]]
dir = "../lib"
publisher = "acme"
name = "lib"
"#,
    );
    let mut ctx = member_context(&dir, "lib");
    let before = std::fs::read_to_string(dir.path().join("lib/model.toml")).unwrap();

    let result = ctx.edit_manifest(|document| {
        edit::push_usage(
            document,
            &usage::MemberUsage::Declared(usage::Usage::SourcePath {
                dir: "../app".into(),
                publisher: crate::model::IndexPublisher::parse("acme").unwrap(),
                name: crate::model::IndexName::parse("app").unwrap(),
            }),
        )
    });
    assert_matches!(result, Err(EditError::Edit(ManifestStep::Workspace(_))));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("lib/model.toml")).unwrap(),
        before,
        "nothing was written"
    );
}

/// A member added to the workspace is checked with the workspace before
/// either file is written.
#[cfg(feature = "filesystem")]
#[test]
fn add_member_writes_the_member_then_the_root() {
    let dir = two_members("");
    let workspace = Workspace::load(dir.path()).unwrap();
    let mut ctx = ProjectContext::new(Some(workspace), None, None);
    let text = std::fs::read_to_string(dir.path().join("lib/model.toml"))
        .unwrap()
        .replace("name = \"lib\"", "name = \"extra\"");

    ctx.add_member("extra", &text, || {
        std::fs::create_dir(dir.path().join("extra")).unwrap();
        std::fs::write(dir.path().join("extra/model.toml"), &text)
    })
    .unwrap();

    assert_eq!(ctx.workspace().unwrap().members().len(), 3);
    assert!(
        std::fs::read_to_string(dir.path().join("model.toml"))
            .unwrap()
            .contains("extra")
    );
    Workspace::load(dir.path()).expect("the workspace loads from disk");
}

/// A member that would duplicate another is refused without writing.
#[cfg(feature = "filesystem")]
#[test]
fn add_member_refuses_a_member_the_workspace_would_not_load_with() {
    use crate::context::AddMemberError;
    let dir = two_members("");
    let workspace = Workspace::load(dir.path()).unwrap();
    let mut ctx = ProjectContext::new(Some(workspace), None, None);
    let before = std::fs::read_to_string(dir.path().join("model.toml")).unwrap();
    // The same project as `lib`.
    let text = std::fs::read_to_string(dir.path().join("lib/model.toml")).unwrap();

    let result = ctx.add_member("extra", &text, || -> Result<(), std::io::Error> {
        panic!("the member is written only after the checks")
    });

    assert_matches!(result, Err(AddMemberError::Load(_)));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("model.toml")).unwrap(),
        before
    );
}

/// The manifest as it was is written back, in the snapshot as well.
#[cfg(feature = "filesystem")]
#[test]
fn restore_manifest_puts_the_manifest_back() {
    use crate::project::ProjectDefinition;
    let dir = two_members("");
    let mut ctx = member_context(&dir, "app");
    let backup = ctx.backup_manifest().unwrap();
    ctx.edit_manifest(|document| {
        edit::set_project_key(document, edit::ProjectKey::Version, "2.0.0".into())
    })
    .unwrap();

    ctx.restore_manifest(&backup).unwrap();

    let Some(ProjectDefinition::Toml(definition)) = ctx.current_definition() else {
        panic!("a `model.toml` definition");
    };
    assert_eq!(definition.version.to_string(), "1.0.0");
    assert!(
        std::fs::read_to_string(dir.path().join("app/model.toml"))
            .unwrap()
            .contains("version = \"1.0.0\"")
    );
}

/// Without a current project there is nothing to edit.
#[cfg(feature = "filesystem")]
#[test]
fn edit_project_needs_a_current_project() {
    use crate::context::EditError;
    let mut ctx = ProjectContext::default();
    assert_matches!(
        ctx.edit_project(|_| Ok::<(), !>(())),
        Err(EditError::NoProject)
    );
}

/// A json edit is checked as reading the result would be, and refused
/// before anything is written.
#[cfg(feature = "filesystem")]
#[test]
fn a_json_edit_that_does_not_validate_is_not_written() {
    use crate::{
        context::{EditError, ManifestEdit, ManifestStep},
        manifest::project::ResolvedProject,
        model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
        project::{ProjectRead as _, local_src::LocalSrcProject},
    };

    struct SwitchFamily;
    impl ManifestEdit for SwitchFamily {
        type Output = ();
        type Error = std::convert::Infallible;

        fn toml(
            self,
            _document: &mut edit::DocumentMut,
            _project: &ResolvedProject,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn json(
            self,
            _project: &LocalSrcProject,
            _info: &mut InterchangeProjectInfoRaw,
            meta: &mut InterchangeProjectMetadataRaw,
        ) -> Result<(), Self::Error> {
            // A Syside project needs a `root`, which this has none of.
            meta.metamodel = Some("https://www.sysand.com/spec/SysML/20250201".to_owned());
            Ok(())
        }
    }

    let dir = camino_tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".project.json"),
        r#"{"name":"p","publisher":"acme","version":"1.0.0","usage":[],"x-extra":1}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join(".meta.json"),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    let project = LocalSrcProject::new_access(dir.path(), None);
    let definition = project.get_project().unwrap();
    let mut ctx = ProjectContext::new(None, Some(project), definition);
    let before = std::fs::read_to_string(dir.path().join(".meta.json")).unwrap();

    let result = ctx.edit(SwitchFamily);

    assert_matches!(result, Err(EditError::Edit(ManifestStep::Invalid(_))));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".meta.json")).unwrap(),
        before
    );
}
