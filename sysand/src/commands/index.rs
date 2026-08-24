// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![expect(clippy::result_large_err, reason = "does not matter here")]

use std::fmt::Display;

use anyhow::Result;
use camino::Utf8Path;

use sysand_core::{
    commands::index::{IndexAddError, IndexInitError, IndexRemoveError, IndexYankError},
    env::utils::ErrorBound,
    index::{RemoveTarget, do_index_add, do_index_init, do_index_remove, do_index_yank},
    utils::format_err,
};

use crate::quote_for_shell;

#[derive(Debug)]
pub struct IndexError<T: ErrorBound> {
    suggestion: Option<String>,
    inner: T,
}

impl<T: ErrorBound> IndexError<T> {
    pub fn new_bare(inner: T) -> Self {
        Self {
            suggestion: None,
            inner,
        }
    }

    pub fn new_with(suggestion: String, inner: T) -> Self {
        Self {
            suggestion: Some(suggestion),
            inner,
        }
    }
}

impl<T: ErrorBound> Display for IndexError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format_err(&self.inner))?;
        if let Some(s) = &self.suggestion {
            write!(f, "\n\n\note: {s}")?;
        }
        Ok(())
    }
}
impl<T: ErrorBound> std::error::Error for IndexError<T> {}

pub fn command_index_init<R: AsRef<Utf8Path>>(
    index_root: R,
) -> Result<(), IndexError<IndexInitError>> {
    match do_index_init(index_root) {
        Ok(()) => Ok(()),
        Err(e) => match &e {
            IndexInitError::AlreadyExists | IndexInitError::WriteError(_) => {
                Err(IndexError::new_bare(e))
            }
        },
    }
}

pub fn command_index_add<I: AsRef<str>, P: AsRef<Utf8Path>, R: AsRef<Utf8Path>>(
    iri: Option<I>,
    kpar_path: P,
    index_root: R,
) -> Result<(), IndexError<IndexAddError>> {
    match do_index_add(iri, kpar_path, index_root) {
        Ok(()) => Ok(()),
        Err(e) => match &e {
            IndexAddError::IndexRootNotFound(root) => Err(IndexError {
                suggestion: Some(format!(
                    "before adding packages, create a new index at `{root}` with:\n\
                    sysand index init --index-root {}",
                    quote_for_shell(root.as_str())
                )),
                inner: e,
            }),
            IndexAddError::NotAnIndex(_)
            | IndexAddError::Io(_)
            | IndexAddError::IoWithCleanupSuccess { .. }
            | IndexAddError::MissingInfo(_)
            | IndexAddError::ProjectRemoved { .. }
            | IndexAddError::MissingMeta(_)
            | IndexAddError::ProjectDigest(_)
            | IndexAddError::ProjectRead(_)
            | IndexAddError::DuplicateVersion { .. }
            | IndexAddError::VersionAlreadyExists { .. }
            | IndexAddError::VersionYanked { .. }
            | IndexAddError::VersionRemoved { .. }
            | IndexAddError::InvalidExistingVersion { .. }
            | IndexAddError::InvalidIri(_)
            | IndexAddError::DuplicateProject { .. }
            | IndexAddError::InvalidProject { .. }
            | IndexAddError::IoWithCleanupFailure { .. } => Err(IndexError::new_bare(e)),
            IndexAddError::InvalidJsonFile { .. } => Err(IndexError::new_with(
                String::from(
                    "either fix the JSON file to be valid or recreate the index\n\
                    from the same kpar archives",
                ),
                e,
            )),
            IndexAddError::VersionHasBuildMetadata { version, .. } => Err(IndexError::new_with(
                format!(
                    "remove the build metadata by running in the original project:\n\
                sysand info version --set {}{}{}{}{REBUILD_KPAR}",
                    version.major, version.minor, version.patch, version.pre
                ),
                e,
            )),
            IndexAddError::InvalidPublisherInProject { .. } => Err(IndexError::new_with(
                format!(
                    "publisher has to follow the following rules:\n\
                - between 3 and 50 ASCII characters long\n\
                - starts and ends with a letter or number\n\
                - may contain single spaces or hyphens (`-`)\n\
                set the publisher that follows the above rules with:
                sysand info publisher --set <publisher>{REBUILD_KPAR}"
                ),
                e,
            )),
            IndexAddError::InvalidNameInProject { .. } => Err(IndexError::new_with(
                format!(
                    "name has to follow the following rules:\n\
                - between 3 and 50 ASCII characters long\n\
                - starts and ends with a letter or number\n\
                - may contain single spaces, dots or hyphens (`-`)\n\
                set the publisher that follows the above rules with:
                sysand info name --set <name>{REBUILD_KPAR}"
                ),
                e,
            )),
            IndexAddError::InconsistentName { .. }
            | IndexAddError::InconsistentPublisher { .. } => Err(IndexError::new_with(
                String::from(
                    "no need to specify the IRI, as it will be derived from\n\
                    the project publisher and name if both follow Sysand naming rules, which are:\n\
                    - between 3 and 50 ASCII characters long\n\
                    - starts and ends with a letter or number\n\
                    - may contain single spaces or hyphens (`-`); name can in addition contain dots",
                ),
                e,
            )),
            IndexAddError::MissingPublisherSpecifiedInIri { iri_publisher, .. } => {
                Err(IndexError::new_with(
                    format!(
                        "set a project publisher with:\n\
                sysand info publisher --set {iri_publisher}{REBUILD_KPAR}"
                    ),
                    e,
                ))
            }
            IndexAddError::MissingPublisherAndIri => Err(IndexError::new_with(
                format!(
                    "set a project publisher with:\n\
                sysand info publisher --set <publisher>{REBUILD_KPAR}"
                ),
                e,
            )),
            IndexAddError::ProjectNotAtRoot { kpar_path, .. } => Err(IndexError::new_with(
                format!(
                    "to fix this, rebuild the kpar with sysand:\n\
                if you have access to the original project, run:
                sysand build\n\
                if not, clone and rebuild the project by running in an empty directory:
                sysand clone {}\n\
                sysand build\n\
                and then try adding the built KPAR to the index",
                    quote_for_shell(kpar_path.as_str())
                ),
                e,
            )),
        },
    }
}

pub fn command_index_yank<I: AsRef<str>, V: AsRef<str>, R: AsRef<Utf8Path>>(
    iri: I,
    version: V,
    index_root: R,
) -> Result<(), IndexError<IndexYankError>> {
    match do_index_yank(iri, version, index_root) {
        Ok(()) => Ok(()),
        Err(e) => match &e {
            IndexYankError::IndexRootNotFound(_)
            | IndexYankError::NotAnIndex { .. }
            | IndexYankError::ProjectNotFound { .. }
            | IndexYankError::Io(_)
            | IndexYankError::InvalidIri(_)
            | IndexYankError::VersionRemoved { .. }
            | IndexYankError::VersionNotFound { .. } => Err(IndexError::new_bare(e)),
            IndexYankError::InvalidJsonFile { .. } => Err(IndexError::new_with(
                String::from(
                    "either fix the JSON file to be valid or recreate the index\n\
                    from the same kpar archives",
                ),
                e,
            )),
        },
    }
}

pub fn command_index_remove<I: AsRef<str>, R: AsRef<Utf8Path>>(
    iri: I,
    target: RemoveTarget,
    index_root: R,
) -> Result<(), IndexError<IndexRemoveError>> {
    match do_index_remove(iri, target, index_root) {
        Ok(()) => Ok(()),
        Err(e) => match &e {
            IndexRemoveError::IndexRootNotFound(_)
            | IndexRemoveError::NotAnIndex { .. }
            | IndexRemoveError::ProjectNotFound { .. }
            | IndexRemoveError::Io(_)
            | IndexRemoveError::InvalidIri(_)
            | IndexRemoveError::VersionNotFound { .. } => Err(IndexError::new_bare(e)),
            IndexRemoveError::InvalidJsonFile { .. } => Err(IndexError::new_with(
                String::from(
                    "either fix the JSON file to be valid or recreate the index\n\
                    from the same kpar archives",
                ),
                e,
            )),
        },
    }
}

const REBUILD_KPAR: &str = "\nand then rebuild the KPAR before adding it to the index";

#[cfg(test)]
#[path = "./index_tests.rs"]
mod tests;
