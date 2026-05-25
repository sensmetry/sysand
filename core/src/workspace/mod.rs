// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

pub mod inheritance;
pub mod resolved_project;
pub mod types;

pub use inheritance::*;
pub use resolved_project::*;
pub use types::*;

use camino::{Utf8Path, Utf8PathBuf};
use serde_json;
use thiserror::Error;

use crate::project::utils::{FsIoError, wrapfs};

#[derive(Debug, Error)]
#[error("workspace deserialization error: {msg}: {err}")]
pub struct WorkspaceDeserializationError {
    msg: &'static str,
    err: serde_json::Error,
}

impl WorkspaceDeserializationError {
    pub fn new(msg: &'static str, err: serde_json::Error) -> Self {
        Self { msg, err }
    }
}

#[derive(Error, Debug)]
pub enum WorkspaceReadError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to deserialize `.workspace.json`: {0}")]
    Deserialize(#[from] WorkspaceDeserializationError),
    #[error("invalid workspace configuration in `{0}`: {1}")]
    Validation(Utf8PathBuf, WorkspaceValidationError),
}

#[derive(Debug)]
pub struct Workspace {
    root_dir: Utf8PathBuf,
    info: WorkspaceInfo,
}

impl Workspace {
    /// Read and parse workspace info file `.workspace.json` residing in `root_dir`
    pub fn new(root_dir: Utf8PathBuf) -> Result<Self, WorkspaceReadError> {
        let info_path = root_dir.join(".workspace.json");
        let raw_info: WorkspaceInfoRaw = serde_json::from_reader(wrapfs::File::open(&info_path)?)
            .map_err(|e| {
            WorkspaceDeserializationError::new("failed to deserialize `.workspace.json`", e)
        })?;
        match WorkspaceInfo::try_from(raw_info) {
            Ok(info) => Ok(Self { root_dir, info }),
            Err(e) => Err(WorkspaceReadError::Validation(info_path, e)),
        }
    }

    pub fn root_path(&self) -> &Utf8Path {
        &self.root_dir
    }

    pub fn info_path(&self) -> Utf8PathBuf {
        self.root_dir.join(".workspace.json")
    }

    pub fn info(&self) -> &WorkspaceInfo {
        &self.info
    }

    pub fn projects(&self) -> &[WorkspaceProjectInfo] {
        &self.info.projects
    }

    pub fn absolute_project_paths(&self) -> Vec<Utf8PathBuf> {
        self.info
            .projects
            .iter()
            .map(|p| self.root_dir.join(&p.path))
            .collect()
    }
}

#[cfg(test)]
mod tests;
