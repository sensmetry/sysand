use std::num::NonZero;

use camino::Utf8PathBuf;
use gix::{prepare_clone, remote::fetch::Shallow};
use thiserror::Error;

use crate::{
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead,
        local_src::{LocalSrcError, LocalSrcProject, PathError},
        utils::{FileWithLifetime, ToPathBuf},
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

    fn sources(&self) -> Vec<Source> {
        vec![Source::RemoteGit {
            remote_git: self.url.to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_imports)]
    use std::{io::Read, process::Command};

    use assert_cmd::prelude::*;
    use tempfile::tempdir;

    use crate::project::{ProjectRead, gix_git_download::GixDownloadedProject};
    //use predicates::prelude::*;

    #[cfg(feature = "alltests")]
    #[test]
    pub fn basic_gix_access() -> Result<(), Box<dyn std::error::Error>> {
        let repo_dir = tempdir()?;
        Command::new("git")
            .arg("init")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        // TODO: Replace by commands::*::do_* when sufficiently complete, also use gix to create repo?
        std::fs::write(
            repo_dir.path().join(".project.json"),
            r#"{"name":"basic_gix_access","version":"1.2.3","usage":[]}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".project.json")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        std::fs::write(
            repo_dir.path().join(".meta.json"),
            r#"{"index":{},"created":"123"}"#,
        )?;
        Command::new("git")
            .arg("add")
            .arg(".meta.json")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        std::fs::write(repo_dir.path().join("test.sysml"), "package Test;")?;
        Command::new("git")
            .arg("add")
            .arg("test.sysml")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg("test_commit")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        Command::new("git")
            .arg("update-server-info")
            .current_dir(repo_dir.path())
            .output()?
            .assert()
            .success();

        // NOTE: Gix does not support the "dumb" HTTP protocol

        // let free_port = port_check::free_local_port().unwrap().to_string();
        // let mut server = Command::new("uv")
        //     .arg("run")
        //     .arg("--isolated")
        //     .arg("--with")
        //     .arg("rangehttpserver")
        //     .arg("-m")
        //     .arg("RangeHTTPServer")
        //     .arg(&free_port)
        //     .current_dir(repo_dir.path().join(".git"))
        //     .spawn()?;

        // sleep(Duration::from_millis(100));

        let project = GixDownloadedProject::new(format!(
            "file://{}",
            repo_dir.path().canonicalize()?.display()
        ))?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!("expected info and meta");
        };

        assert_eq!(info.name, "basic_gix_access");
        assert_eq!(meta.created, "123");

        let mut buf = "".to_string();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut buf)?;
        assert_eq!(buf, "package Test;");

        // server.kill()?;
        Ok(())
    }
}
