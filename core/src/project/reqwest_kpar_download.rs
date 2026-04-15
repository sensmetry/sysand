// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{self, Write as _},
    marker::Unpin,
    pin::Pin,
    sync::Arc,
};

use futures::AsyncRead;
use sha2::{Digest as _, Sha256};
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
    #[error("kpar at `{url}` has sha256 `{actual}` but the expected digest was `{expected}`")]
    DigestMismatch {
        url: Box<str>,
        expected: String,
        actual: String,
    },
}

impl From<FsIoError> for ReqwestKparDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

/// Sibling staging path used for atomic-rename downloads. Sits next to the
/// final path inside the same tempdir so `rename` is a cheap same-filesystem
/// operation.
fn staging_path_for(final_path: &camino::Utf8Path) -> camino::Utf8PathBuf {
    let file_name = final_path.file_name().unwrap_or("project.kpar");
    let staging_name = format!("{file_name}.download");
    match final_path.parent() {
        Some(parent) => parent.join(staging_name),
        None => camino::Utf8PathBuf::from(staging_name),
    }
}

impl<Policy: HTTPAuthentication> ReqwestKparDownloadedProject<Policy> {
    pub fn new(
        url: reqwest::Url,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        Ok(Self {
            url,
            inner: LocalKParProject::new_temporary()?,
            client,
            auth_policy,
        })
    }

    pub fn new_guess_root<S: AsRef<str>>(
        url: S,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        Self::new(
            reqwest::Url::parse(url.as_ref())
                .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?,
            client,
            auth_policy,
        )
    }

    pub async fn ensure_downloaded_with_sha256_digest(
        &self,
        expected_digest: Option<&str>,
    ) -> Result<(), ReqwestKparDownloadedError> {
        if self.inner.archive_path.is_file() {
            return Ok(());
        }

        // Download to a sibling staging path and atomic-rename on success.
        // `archive_path.is_file()` is then the "verified" sentinel: a failed
        // verification leaves only the staging file (best-effort cleaned up),
        // never the final path, so a retry can never serve tampered bytes
        // even if cleanup itself fails.
        let final_path = &self.inner.archive_path;
        let staging_path = staging_path_for(final_path);
        let mut file = wrapfs::File::create(&staging_path)?;

        let resp = self
            .auth_policy
            .with_authentication(&self.client, &kpar_get_request(self.url.clone()))
            .await?;

        if !resp.status().is_success() {
            let _ = std::fs::remove_file(&staging_path);
            return Err(ReqwestKparDownloadedError::BadHttpStatus {
                url: self.url.as_str().into(),
                status: resp.status(),
            });
        }
        let mut bytes_stream = resp.bytes_stream();

        use futures::StreamExt as _;

        let expected_digest = expected_digest.map(ToOwned::to_owned);
        let mut hasher = expected_digest.as_ref().map(|_| Sha256::new());

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = match bytes {
                Ok(b) => b,
                Err(e) => {
                    let _ = std::fs::remove_file(&staging_path);
                    return Err(ReqwestKparDownloadedError::Reqwest(e));
                }
            };
            if let Some(h) = hasher.as_mut() {
                h.update(&bytes);
            }
            if let Err(e) = file.write_all(&bytes) {
                let _ = std::fs::remove_file(&staging_path);
                return Err(FsIoError::WriteFile(staging_path.clone(), e).into());
            }
        }

        // Verify before any sync_all/rename so a mismatched archive never
        // gets durably installed at `final_path`.
        if let (Some(h), Some(expected)) = (hasher, expected_digest.as_ref()) {
            let actual = format!("{:x}", h.finalize());
            if &actual != expected {
                let _ = std::fs::remove_file(&staging_path);
                return Err(ReqwestKparDownloadedError::DigestMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        if let Err(e) = file.sync_all() {
            let _ = std::fs::remove_file(&staging_path);
            return Err(FsIoError::WriteFile(staging_path.clone(), e).into());
        }
        drop(file);

        wrapfs::rename(&staging_path, final_path)?;

        Ok(())
    }

    pub async fn ensure_downloaded(&self) -> Result<(), ReqwestKparDownloadedError> {
        self.ensure_downloaded_with_sha256_digest(None).await
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
