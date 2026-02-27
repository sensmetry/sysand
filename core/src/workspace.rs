use camino::{Utf8Path, Utf8PathBuf};
use fluent_uri::Iri;

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

#[derive(Error, Debug)]
pub enum WorkspaceValidationError {
    #[error("failed to parse `{0}` as IRI: {1}")]
    InvalidIri(String, fluent_uri::ParseError),
}

#[derive(Eq, Clone, PartialEq, Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "python", derive(FromPyObject, IntoPyObject))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfoG<Iri> {
    pub projects: Vec<WorkspaceProjectInfoG<Iri>>,
}

pub type WorkspaceInfoRaw = WorkspaceInfoG<String>;
pub type WorkspaceInfo = WorkspaceInfoG<Iri<String>>;
pub type WorkspaceProjectInfoRaw = WorkspaceProjectInfoG<String>;
pub type WorkspaceProjectInfo = WorkspaceProjectInfoG<Iri<String>>;

impl TryFrom<WorkspaceInfoRaw> for WorkspaceInfo {
    type Error = WorkspaceValidationError;

    fn try_from(value: WorkspaceInfoRaw) -> Result<Self, Self::Error> {
        let mut projects = Vec::with_capacity(value.projects.len());
        for project in value.projects {
            let mut iris = Vec::with_capacity(project.iris.len());
            for iri in project.iris {
                let iri = Iri::parse(iri)
                    .map_err(|(e, iri)| WorkspaceValidationError::InvalidIri(iri, e))?;
                iris.push(iri);
            }
            projects.push(WorkspaceProjectInfo {
                path: project.path,
                iris,
            });
        }

        Ok(Self { projects })
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
}
