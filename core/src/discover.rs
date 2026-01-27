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

/// If current directory is known by caller, consider using `discover_project`
pub fn current_project() -> Result<Option<LocalSrcProject>, Box<FsIoError>> {
    Ok(discover_project(wrapfs::current_dir()?))
}

pub fn discover_project<P: AsRef<Utf8Path>>(working_directory: P) -> Option<LocalSrcProject> {
    let path = discover(working_directory, |path| {
        path.join(".project.json").is_file() || path.join(".meta.json").is_file()
    })?;
    Some(LocalSrcProject { project_path: path })
}

/// If current directory is known by caller, consider using `discover_workspace`
pub fn current_workspace() -> Result<Result<Option<Workspace>, WorkspaceReadError>, Box<FsIoError>>
{
    Ok(discover_workspace(wrapfs::current_dir()?))
}

/// Tries to find workspace in `working_directory` or its ancestors.
/// If found, returns result of reading the workspace info file
pub fn discover_workspace<P: AsRef<Utf8Path>>(
    working_directory: P,
) -> Result<Option<Workspace>, WorkspaceReadError> {
    let path = match discover(working_directory, |path| {
        path.join(".workspace.json").is_file()
    }) {
        Some(p) => p,
        None => return Ok(None),
    };
    Some(Workspace::new(path)).transpose()
}

// TODO: Improve the logic here, this is probably too simple
/// Discover a directory that satisfies `predicate`. Tries
/// `working_directory` and all its ancestors.
fn discover<P: AsRef<Utf8Path>, F: Fn(&Utf8Path) -> bool>(
    working_directory: P,
    predicate: F,
) -> Option<Utf8PathBuf> {
    let mut current = working_directory.to_path_buf();

    log::debug!("trying to discover project in `{}`", current);

    while !predicate(&current) {
        match current.parent() {
            Some(parent) if parent.as_str().is_empty() => {
                log::debug!("hit empty relative path, trying to canonicalise");
                match current.canonicalize_utf8() {
                    Ok(current_canonical) => match current_canonical.parent() {
                        Some(parent_canonical) => current = parent_canonical.to_path_buf(),
                        None => {
                            log::debug!(
                                "canonicalised path `{}` has no parent either",
                                current_canonical
                            );
                            return None;
                        }
                    },
                    Err(e) => {
                        log::debug!("unable to canonicalise path `{}`: {e}", current);
                    }
                }
            }
            Some(parent) => {
                log::debug!("checking for project in parent of `{}`", current);
                current = parent.to_path_buf();
            }
            None => {
                return None;
            }
        }
    }

    Some(current)
}
