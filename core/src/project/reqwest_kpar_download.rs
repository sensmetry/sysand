// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::Write as _;

use tempfile::tempdir;
use thiserror::Error;

use crate::project::{
    ProjectRead,
    local_kpar::{LocalKParError, LocalKParProject},
};

use super::utils::{FsIoError, ToDisplay, wrapfs};

/// Project stored at a remote URL such as https://www.example.com/project.kpar.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// Downloads the full archive to a temporary directory and then accesses it using
/// `LocalKParProject`.
#[derive(Debug)]
pub struct ReqwestKparDownloadedProject {
    pub url: reqwest::Url,
    pub inner: LocalKParProject,
}

#[derive(Error, Debug)]
pub enum ReqwestKparDownloadedError {
    #[error("Address '{0}' not found: {1}")]
    UnableToAccess(reqwest::Url, reqwest::StatusCode),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    KPar(#[from] LocalKParError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl From<FsIoError> for ReqwestKparDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl ReqwestKparDownloadedProject {
    pub fn new_guess_root<S: AsRef<str>>(url: S) -> Result<Self, ReqwestKparDownloadedError> {
        let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;

        Ok(ReqwestKparDownloadedProject {
            url: reqwest::Url::parse(url.as_ref())?,
            inner: LocalKParProject {
                archive_path: wrapfs::canonicalize(tmp_dir.path())?
                    .join("project.kpar")
                    .to_path_buf(),
                tmp_dir,
                root: None,
            },
        })
    }

    pub fn ensure_downloaded(&self) -> Result<(), ReqwestKparDownloadedError> {
        if self.inner.archive_path.is_file() {
            return Ok(());
        }

        let mut file = wrapfs::File::create(&self.inner.archive_path)?;

        let resp = reqwest::blocking::get(self.url.clone())?;

        if !resp.status().is_success() {
            return Err(ReqwestKparDownloadedError::UnableToAccess(
                self.url.clone(),
                resp.status(),
            ));
        }

        file.write_all(&resp.bytes()?)
            .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.to_display(), e))?;

        file.flush()
            .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.to_display(), e))?;

        Ok(())
    }
}

impl ProjectRead for ReqwestKparDownloadedProject {
    type Error = ReqwestKparDownloadedError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.ensure_downloaded()?;

        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = <LocalKParProject as ProjectRead>::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded()?;

        Ok(self.inner.read_source(path)?)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::RemoteKpar {
            remote_kpar: self.url.to_string(),
            remote_kpar_size: self.inner.file_size().ok(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write as _};

    use crate::project::ProjectRead;

    #[test]
    fn test_basic_download_request() -> Result<(), Box<dyn std::error::Error>> {
        let buf = {
            let mut cursor = std::io::Cursor::new(vec![]);
            let mut zip = zip::ZipWriter::new(&mut cursor);

            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);

            zip.start_file("some_root_dir/.project.json", options)?;
            zip.write_all(
                br#"{"name":"test_basic_download_request","version":"1.2.3","usage":[]}"#,
            )?;
            zip.start_file("some_root_dir/.meta.json", options)?;
            zip.write_all(br#"{"index":{},"created":"123"}"#)?;
            zip.start_file("some_root_dir/test.sysml", options)?;
            zip.write_all(br#"package Test;"#)?;

            zip.finish().unwrap();

            cursor.flush()?;
            cursor.into_inner()
        };

        let mut server = mockito::Server::new();

        //let host = server.host_with_port();
        let url = reqwest::Url::parse(&server.url()).unwrap();

        let get_kpar = server
            .mock("GET", "/test_basic_download_request.kpar")
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(&buf)
            .create();

        let project = super::ReqwestKparDownloadedProject::new_guess_root(format!(
            "{}test_basic_download_request.kpar",
            url,
        ))?;

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!()
        };

        assert_eq!(info.name, "test_basic_download_request");
        assert_eq!(meta.created, "123");

        let mut src = "".to_string();
        project
            .read_source("test.sysml")?
            .read_to_string(&mut src)?;

        assert_eq!(src, "package Test;");

        get_kpar.assert();

        Ok(())
    }
}
