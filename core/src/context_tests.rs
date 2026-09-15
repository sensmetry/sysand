// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use super::ProjectContext;

// ------------------- the sources root

/// `workspaces.md` "Locking": one lockfile for the whole workspace at
/// its root, recording "every project of the solved graph ... members
/// included". Every `src_path` in it is therefore relative to that
/// root, not to whichever member happened to run `lock`.
#[cfg(feature = "filesystem")]
#[test]
fn the_model_workspace_root_wins_as_the_sources_root() {
    let ctx = ProjectContext {
        model_workspace_root: Some("/ws".into()),
        current_project: Some(crate::project::local_src::LocalSrcProject::new_access(
            "/ws/app", None,
        )),
        ..Default::default()
    };
    assert_eq!(
        ctx.sources_root().as_deref(),
        Some(camino::Utf8Path::new("/ws"))
    );
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

/// And with neither there is no root -- which is what makes
/// `LocalSrcProject::sources` panic, deliberately, rather than invent
/// one.
#[cfg(feature = "filesystem")]
#[test]
fn with_neither_there_is_no_sources_root() {
    assert_eq!(ProjectContext::default().sources_root(), None);
}
