// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use camino::{Utf8Path, Utf8PathBuf};
use typed_path::Utf8UnixPathBuf;

use crate::{
    manifest::{MODEL_TOML_FILE, ModelToml, ModelTomlKind},
    project::{
        local_src::LocalSrcProject,
        utils::{FsIoError, ToPathBuf as _, wrapfs},
    },
    workspace::{Workspace, WorkspaceReadError},
};

/// Whether `path` is a project directory.
///
/// A json project is one that holds either of its two manifests. A
/// `model.toml` needs reading, because the same file name is used for
/// workspace configuration: a workspace root defines no project, and
/// stopping the upward walk there would answer a question nobody asked.
/// [`ModelToml::peek`] is that question and nothing more -- it parses the
/// document and looks for the two table names, without validating either.
///
/// A `model.toml` that is not TOML, or that defines neither table, counts
/// as *found*. The file is this directory's manifest and it is broken;
/// walking past it to a parent would answer a different directory's
/// question and bury the error.
fn is_project_file(path: &Utf8Path) -> Result<bool, Box<FsIoError>> {
    let model_toml = path.join(MODEL_TOML_FILE);
    if wrapfs::is_file(&model_toml)? {
        let text = wrapfs::read_to_string(&model_toml)?;
        return Ok(!matches!(
            ModelToml::peek(MODEL_TOML_FILE, &text),
            Ok(Some(ModelTomlKind::Workspace))
        ));
    }
    Ok(wrapfs::is_file(path.join(".project.json"))? || wrapfs::is_file(path.join(".meta.json"))?)
}

pub fn discover_project<P: AsRef<Utf8Path>>(
    working_directory: P,
) -> Result<Option<LocalSrcProject>, Box<FsIoError>> {
    log::debug!(
        "trying to discover project in `{}`",
        working_directory.as_ref()
    );
    // FIXME: assuming nominal path "." is incorrect
    let project = discover(working_directory, is_project_file)?
        .map(|path| LocalSrcProject::new_access(path, Some(Utf8UnixPathBuf::from("."))));
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
    let Some(path) = discover(working_directory, |path| {
        wrapfs::is_file(path.join(".workspace.json"))
    })?
    else {
        return Ok(None);
    };
    Some(Workspace::new(path)).transpose()
}

#[cfg(test)]
#[path = "./discover_tests.rs"]
mod tests;

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
                match wrapfs::canonicalize_raw(&current) {
                    Ok(current_canonical) => {
                        if let Some(parent_canonical) = current_canonical.parent() {
                            current = parent_canonical.to_path_buf()
                        } else {
                            log::debug!(
                                "discover: canonicalized path `{current_canonical}` has no parent either"
                            );
                            return Ok(None);
                        }
                    }
                    Err(e) => {
                        log::debug!("discover: unable to canonicalize path `{current}`: {e}");
                    }
                }
            }
            Some(parent) => {
                log::debug!("discover: checking in `{parent}`");
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
