use camino::Utf8PathBuf;
#[cfg(feature = "python")]
use pyo3::{FromPyObject, IntoPyObject};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::project::utils::{FsIoError, wrapfs};

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProjectInfoG<Iri> {
    pub path: String,
    pub iris: Vec<Iri>,
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfoG<Iri> {
    pub projects: Vec<WorkspaceProjectInfoG<Iri>>,
}

pub type WorkspaceInfoRaw = WorkspaceInfoG<String>;
pub type WorkspaceProjectInfoRaw = WorkspaceProjectInfoG<String>;

#[derive(Error, Debug)]
pub enum WorkspaceReadError {
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("failed to deserialize `.workspace.json`: {0}")]
    Deserialize(#[from] WorkspaceDeserializationError),
}

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

pub struct Workspace {
    pub workspace_path: Utf8PathBuf,
}

impl Workspace {
    pub fn root_path(&self) -> Utf8PathBuf {
        self.workspace_path.clone()
    }

    pub fn info_path(&self) -> Utf8PathBuf {
        self.workspace_path.join(".workspace.json")
    }

    pub fn get_info(&self) -> Result<Option<WorkspaceInfoRaw>, WorkspaceReadError> {
        let info_json_path = self.info_path();

        let info_json = if info_json_path.exists() {
            Some(
                serde_json::from_reader(wrapfs::File::open(&info_json_path)?).map_err(|e| {
                    WorkspaceDeserializationError::new("failed to deserialize `.workspace.json`", e)
                })?,
            )
        } else {
            None
        };

        Ok(info_json)
    }

    pub fn get_projects(&self) -> Result<Option<Vec<WorkspaceProjectInfoRaw>>, WorkspaceReadError> {
        let info = self.get_info()?;
        Ok(info.map(|info| info.projects))
    }
}
