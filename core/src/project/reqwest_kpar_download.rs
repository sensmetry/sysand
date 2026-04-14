// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{self, Write as _},
    marker::Unpin,
    pin::Pin,
    sync::Arc,
};

use futures::AsyncRead;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{
        ProjectRead, ProjectReadAsync,
        local_kpar::{LocalKParError, LocalKParProject},
    },
    resolve::net_utils::kpar_get_request,
};

use super::utils::{FsIoError, wrapfs};

/// Project stored at a remote URL such as https://www.example.com/project.kpar.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// Downloads the full archive to a temporary directory and then accesses it using
/// `LocalKParProject`.
#[derive(Debug)]
pub struct ReqwestKparDownloadedProject<Policy> {
    pub url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub inner: LocalKParProject,
    pub auth_policy: Arc<Policy>,
}

// TODO: reduce size of errors here and elsewhere
#[derive(Error, Debug)]
pub enum ReqwestKparDownloadedError {
    #[error("HTTP request to `{url}` returned status {status}")]
    BadHttpStatus {
        url: Box<str>,
        status: reqwest::StatusCode,
    },
    #[error("failed to parse URL `{0}`: {1}")]
    ParseUrl(Box<str>, url::ParseError),
    // TODO: nicer formatting. Debug formatting is used here to include
    // all the details, since they are not given in the Display impl
    #[error("error making an HTTP request:\n{0:#?}")]
    Reqwest(reqwest::Error),
    #[error("error making an HTTP request:\n{0:#?}")]
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

impl<Policy: HTTPAuthentication> ReqwestKparDownloadedProject<Policy> {
    pub fn new_guess_root<S: AsRef<str>>(
        url: S,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        Ok(ReqwestKparDownloadedProject {
            url: reqwest::Url::parse(url.as_ref())
                .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?,
            inner: LocalKParProject::new_temporary()?,
            client,
            auth_policy,
        })
    }

    pub async fn ensure_downloaded(&self) -> Result<(), ReqwestKparDownloadedError> {
        if self.inner.archive_path.is_file() {
            return Ok(());
        }

        let mut file = wrapfs::File::create(&self.inner.archive_path)?;

        let resp = self
            .auth_policy
            .with_authentication(&self.client, &kpar_get_request(self.url.clone()))
            .await?;

        if !resp.status().is_success() {
            return Err(ReqwestKparDownloadedError::BadHttpStatus {
                url: self.url.as_str().into(),
                status: resp.status(),
            });
        }
        let mut bytes_stream = resp.bytes_stream();

        use futures::StreamExt as _;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(ReqwestKparDownloadedError::Reqwest)?;
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.clone(), e))?;
        }

        file.sync_all()
            .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.clone(), e))?;

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

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestKparDownloadedProject<Policy> {
    type Error = ReqwestKparDownloadedError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
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

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::RemoteKpar {
            remote_kpar: self.url.to_string(),
            remote_kpar_size: self.inner.file_size().ok(),
        }])
    }
}

#[cfg(test)]
#[path = "./reqwest_kpar_download_tests.rs"]
mod tests;
