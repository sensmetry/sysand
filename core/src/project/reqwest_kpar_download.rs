// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    io::{self, Write as _},
    marker::Unpin,
    pin::Pin,
    sync::Arc,
};

use futures::AsyncRead;
use reqwest_middleware::{ClientWithMiddleware, RequestBuilder};
use tempfile::tempdir;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    project::{
        ProjectRead, ProjectReadAsync,
        local_kpar::{LocalKParError, LocalKParProject},
    },
};

use super::utils::{FsIoError, ToPathBuf, wrapfs};

/// Project stored at a remote URL such as https://www.example.com/project.kpar.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// Downloads the full archive to a temporary directory and then accesses it using
/// `LocalKParProject`.
#[derive(Debug)]
pub struct ReqwestKparDownloadedProject<Pol> {
    pub url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub inner: LocalKParProject,
    pub auth_policy: Arc<Pol>,
}

#[derive(Error, Debug)]
pub enum ReqwestKparDownloadedError {
    #[error("HTTP request to `{0}` returned status {1}")]
    BadHttpStatus(reqwest::Url, reqwest::StatusCode),
    #[error("failed to parse URL `{0}`: {1}")]
    ParseUrl(Box<str>, url::ParseError),
    #[error("HTTP request to `{0}` failed: {1}")]
    Reqwest(Box<str>, reqwest_middleware::Error),
    #[error("failed to decode data received from HTTP request `{0}`: {1}")]
    ResponseDecode(Box<str>, reqwest_middleware::Error),
    #[error(transparent)]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
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

impl<Pol: HTTPAuthentication> ReqwestKparDownloadedProject<Pol> {
    pub fn new_guess_root<S: AsRef<str>>(
        url: S,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Pol>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;

        Ok(ReqwestKparDownloadedProject {
            url: reqwest::Url::parse(url.as_ref())
                .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?,
            inner: LocalKParProject {
                archive_path: {
                    let mut p = wrapfs::canonicalize(tmp_dir.path())?;
                    p.push("project.kpar");
                    p
                },
                tmp_dir,
                root: None,
            },
            client,
            auth_policy,
        })
    }

    pub async fn ensure_downloaded(&self) -> Result<(), ReqwestKparDownloadedError> {
        if self.inner.archive_path.is_file() {
            return Ok(());
        }

        let mut file = wrapfs::File::create(&self.inner.archive_path)?;

        let this_url = self.url.clone();
        let resp = self
            .auth_policy
            .with_authentication(
                &self.client,
                &move |client: &ClientWithMiddleware| -> RequestBuilder {
                    client.get(this_url.clone())
                },
            )
            .await
            .map_err(|e| ReqwestKparDownloadedError::Reqwest(self.url.as_str().into(), e))?;

        if !resp.status().is_success() {
            return Err(ReqwestKparDownloadedError::BadHttpStatus(
                self.url.clone(),
                resp.status(),
            ));
        }
        let mut bytes_stream = resp.bytes_stream();

        use futures::StreamExt as _;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(|e| {
                ReqwestKparDownloadedError::Reqwest(self.url.as_str().to_string().into(), e.into())
            })?;
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.to_path_buf(), e))?;
        }

        file.flush()
            .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.to_path_buf(), e))?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct AsAsyncRead<T> {
    pub inner: T,
}

impl<T: io::Read + Unpin> AsyncRead for AsAsyncRead<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(self.get_mut().inner.read(buf))
    }
}

impl<Pol: HTTPAuthentication> ProjectReadAsync for ReqwestKparDownloadedProject<Pol> {
    type Error = ReqwestKparDownloadedError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.ensure_downloaded().await?;

        Ok(self.inner.get_project()?)
    }

    type SourceReader<'a>
        = AsAsyncRead<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded().await?;

        Ok(AsAsyncRead {
            inner: self.inner.read_source(path)?,
        })
    }

    async fn sources_async(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::RemoteKpar {
            remote_kpar: self.url.to_string(),
            remote_kpar_size: self.inner.file_size().ok(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write as _},
        sync::Arc,
    };

    use crate::{
        auth::Unauthenticated,
        project::{ProjectRead, ProjectReadAsync},
    };

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

        let project = super::ReqwestKparDownloadedProject::new_guess_root(
            format!("{}test_basic_download_request.kpar", url,),
            reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            Arc::new(Unauthenticated {}),
        )?
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        ));

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
