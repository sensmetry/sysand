use std::num::NonZero;

use camino::Utf8PathBuf;
use gix::{prepare_clone, remote::fetch::Shallow};
use thiserror::Error;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead,
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{FileWithLifetime, RelativizePathError, ToPathBuf},
    },
};

use super::utils::{FsIoError, ProjectDeserializationError, ProjectSerializationError, wrapfs};

#[derive(Debug)]
pub struct GixDownloadedProject {
    pub url: gix::Url,
    tmp_dir: camino_tempfile::Utf8TempDir,
    inner: LocalSrcProject,
}

#[derive(Error, Debug)]
pub enum GixDownloadedError {
    #[error("git clone from `{0}` failed: {1}")]
    Clone(String, Box<gix::clone::Error>),
    #[error("failed to parse git URL `{0}`: {1}")]
    UrlParse(Box<str>, Box<gix::url::parse::Error>),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error(transparent)]
    Path(#[from] PathError),
    #[error(transparent)]
    Deserialize(#[from] ProjectDeserializationError),
    #[error(transparent)]
    Serialize(#[from] ProjectSerializationError),
    #[error("git fetch from `{0}` failed: {1}")]
    Fetch(String, Box<gix::clone::fetch::Error>),
    #[error("git checkout in temporary directory `{0}` failed: {1}")]
    Checkout(Utf8PathBuf, Box<gix::clone::checkout::main_worktree::Error>),
    #[error(
        "cannot construct a relative path from the workspace/project
        directory to one of its dependencies' directory:\n\
        {0}"
    )]
    ImpossibleRelativePath(#[from] RelativizePathError),
    #[error("{0}")]
    Other(String),
}

impl From<FsIoError> for GixDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl From<LocalSrcError> for GixDownloadedError {
    fn from(value: LocalSrcError) -> Self {
        match value {
            LocalSrcError::Deserialize(error) => Self::Deserialize(error),
            LocalSrcError::Path(error) => Self::Path(error),
            LocalSrcError::AlreadyExists(msg) => {
                GixDownloadedError::Other(format!("unexpected internal error: {}", msg))
            }
            LocalSrcError::Io(e) => Self::Io(e),
            LocalSrcError::Serialize(error) => Self::Serialize(error),
            LocalSrcError::ImpossibleRelativePath(err) => Self::ImpossibleRelativePath(err),
            LocalSrcError::MissingMeta => GixDownloadedError::Other(
                "project is missing metadata file `.meta.json`".to_string(),
            ),
        }
    }
}

impl GixDownloadedProject {
    pub fn new<S: AsRef<str>>(url: S) -> Result<GixDownloadedProject, GixDownloadedError> {
        let tmp_dir = camino_tempfile::tempdir().map_err(FsIoError::MkTempDir)?;

        Ok(GixDownloadedProject {
            url: gix::url::parse(url.as_ref().into())
                .map_err(|e| GixDownloadedError::UrlParse(url.as_ref().into(), Box::new(e)))?,
            inner: LocalSrcProject {
                nominal_path: None,
                project_path: wrapfs::canonicalize(tmp_dir.path())?,
            },
            tmp_dir,
        })
    }

    fn ensure_downloaded(&self) -> Result<(), GixDownloadedError> {
        if !self.tmp_dir.path().join(".git").is_dir() {
            let prepared_clone = prepare_clone(self.url.clone(), self.tmp_dir.path())
                .map_err(|e| GixDownloadedError::Clone(self.url.to_string(), Box::new(e)))?;

            let (mut prepare_checkout, _) = prepared_clone
                .with_shallow(Shallow::DepthAtRemote(NonZero::new(1).unwrap()))
                .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .map_err(|e| GixDownloadedError::Fetch(self.url.to_string(), Box::new(e)))?;
            let (_repo, _) = prepare_checkout
                .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
                .map_err(|e| {
                    GixDownloadedError::Checkout(self.tmp_dir.to_path_buf(), Box::new(e))
                })?;
        }

        Ok(())
    }
}

impl ProjectRead for GixDownloadedProject {
    type Error = GixDownloadedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.ensure_downloaded()?;

        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = FileWithLifetime<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded()?;

        Ok(FileWithLifetime::new(self.inner.read_source(path)?))
    }

    fn sources(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::RemoteGit {
            remote_git: self.url.to_string(),
        }])
    }
}

#[cfg(test)]
#[path = "./gix_git_download_tests.rs"]
mod tests;
