// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    project::{
        local_src::LocalSrcProject,
        utils::{FsIoError, ToPathBuf, wrapfs},
    },
    workspace::{Workspace, WorkspaceReadError},
};

fn is_project_file(path: &Utf8Path) -> Result<bool, Box<FsIoError>> {
    Ok(wrapfs::is_file(path.join(".project.json"))? || wrapfs::is_file(path.join(".meta.json"))?)
}

pub fn discover_project<P: AsRef<Utf8Path>>(
    working_directory: P,
) -> Result<Option<LocalSrcProject>, Box<FsIoError>> {
    log::debug!(
        "trying to discover project in `{}`",
        working_directory.as_ref()
    );
    let project = discover(working_directory, is_project_file)?.map(|path| LocalSrcProject {
        nominal_path: Some(Utf8PathBuf::from(".")),
        project_path: path,
    });
    Ok(project)
}

/// Tries to find workspace in `working_directory` or its ancestors.
/// If found, returns result of reading the workspace info file
pub fn discover_workspace<P: AsRef<Utf8Path>>(
    working_directory: P,
) -> Result<Option<Workspace>, WorkspaceReadError> {
    log::debug!(
        "trying to discover workspace in `{}`",
        working_directory.as_ref()
    );
    let path = match discover(working_directory, |path| {
        wrapfs::is_file(path.join(".workspace.json"))
    })? {
        Some(p) => p,
        None => return Ok(None),
    };
    Some(Workspace::new(path)).transpose()
}

/// Discover a directory that satisfies `predicate`. Tries
/// `working_directory` and all its ancestors.
fn discover<P: AsRef<Utf8Path>, F: Fn(&Utf8Path) -> Result<bool, Box<FsIoError>>>(
    working_directory: P,
    predicate: F,
) -> Result<Option<Utf8PathBuf>, Box<FsIoError>> {
    let mut current = working_directory.to_path_buf();

    while !predicate(&current)? {
        match current.parent() {
            Some(parent) if parent.as_str().is_empty() => {
                log::debug!("discover: hit empty relative path, trying to canonicalize");
                match current.canonicalize_utf8() {
                    Ok(current_canonical) => match current_canonical.parent() {
                        Some(parent_canonical) => current = parent_canonical.to_path_buf(),
                        None => {
                            log::debug!(
                                "discover: canonicalized path `{}` has no parent either",
                                current_canonical
                            );
                            return Ok(None);
                        }
                    },
                    Err(e) => {
                        log::debug!("discover: unable to canonicalize path `{}`: {e}", current);
                    }
                }
            }
            Some(parent) => {
                log::debug!("discover: checking in parent of `{}`", current);
                current = parent.to_path_buf();
            }
            None => {
                log::debug!("discover: not found");
                return Ok(None);
            }
        }
    }
    log::debug!("discover: found in `{current}`");

    Ok(Some(current))
}
