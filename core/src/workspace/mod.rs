// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Workspaces.
//!
//! A workspace is a `model.toml` holding a `[workspace]` table; see
//! [`model_toml`].

use camino::Utf8PathBuf;

use crate::discover::{Ancestry, DirManifest, ManifestCache, Seek};
use crate::project::{
    ProjectFormat,
    local_src::{LocalSrcProject, json_manifest},
};

pub mod model_toml;

use model_toml::{DiscoverError, PathRegion};

/// A [`LocalSrcProject`] for a path a caller gave, rather than a
/// directory that was walked to.
///
/// A member's manifest resolves only against its workspace's config, so
/// this finds the workspace for a path the way discovery does for a
/// working directory. The walk's own read of the path's `model.toml` is
/// what a member is resolved from.
///
/// A command with a [`crate::context::ProjectContext`] uses
/// [`crate::context::ProjectContext::project_at_path`], which takes a
/// member of the workspace it loaded as it is.
///
/// Built from [`model_toml::table_of`] and [`model_toml::region_of`], which
/// matches a member's directory exactly, so a project merely inside one is
/// not handed its workspace.
///
/// An excluded project, one outside any workspace and a json project that
/// is not listed read standalone. An unlisted `model.toml` project is
/// refused, as when walking up from inside it, and so is a listed member
/// that is a json project or holds both manifests; see
/// [`model_toml::PathRegion`].
pub fn project_at_path(
    path: impl Into<Utf8PathBuf>,
    nominal_path: Option<typed_path::Utf8UnixPathBuf>,
) -> Result<LocalSrcProject, DiscoverError> {
    project_at_path_cached(path, nominal_path, &mut ManifestCache::default())
}

/// The project at `path`, reading each directory's manifest once per
/// `cache`, for paths whose walks share directories.
pub fn project_at_path_cached(
    path: impl Into<Utf8PathBuf>,
    nominal_path: Option<typed_path::Utf8UnixPathBuf>,
    cache: &mut ManifestCache,
) -> Result<LocalSrcProject, DiscoverError> {
    let path = path.into();
    let ancestry = Ancestry::walk_cached(&path, Seek::Workspace, cache)?;
    let canonical = ancestry.start().to_owned();
    let own = ancestry.start_manifest().cloned();
    let Some((root, config)) = model_toml::table_of(ancestry)? else {
        return Ok(LocalSrcProject::new_access(path, nominal_path));
    };
    let json = json_manifest(&canonical)?;
    let has_toml = matches!(own, Some(DirManifest::ModelToml { file: _, parsed: _ }));
    let format = ProjectFormat::from_presence(has_toml, json.is_some());
    match model_toml::project_region(&root, &config, &canonical, format) {
        PathRegion::Member(index, _) => match (own, json) {
            (Some(DirManifest::ModelToml { file, parsed }), None) => {
                let resolved = config
                    .resolve_member_parsed(index, file.as_str(), parsed.manifest)
                    .map_err(DiscoverError::Manifest)?;
                Ok(LocalSrcProject::new_member(path, nominal_path, resolved))
            }
            (Some(DirManifest::ModelToml { file: _, parsed: _ }), Some(json)) => {
                Err(DiscoverError::AmbiguousMember {
                    project: canonical,
                    json,
                })
            }
            (_, Some(json)) => Err(DiscoverError::JsonMember {
                project: canonical,
                json,
            }),
            // A directory that defines no project is handed over unread,
            // for its reader to report.
            (_, None) => Ok(LocalSrcProject::new_access(path, nominal_path)),
        },
        PathRegion::Excluded | PathRegion::Outside => {
            Ok(LocalSrcProject::new_access(path, nominal_path))
        }
        PathRegion::Unlisted => Err(DiscoverError::Unlisted {
            project: canonical,
            root,
        }),
    }
}
