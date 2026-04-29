// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{self, Write as _},
    num::NonZeroU64,
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
    inner: LocalKParProject,
    pub auth_policy: Arc<Policy>,
    /// Optional expected sha256 hex digest for callers that need this
    /// instance to enforce archive verification before exposing project
    /// contents (e.g. lockfile-driven `sync`).
    expected_sha256_hex: Option<String>,
    /// Optional expected archive byte length. Index-backed kpars carry this
    /// in the lockfile / versions index; enforce it while streaming so a
    /// malicious server cannot exhaust disk before the digest check fails.
    expected_size: Option<NonZeroU64>,
    /// Fans concurrent `ensure_downloaded*` calls on the same instance
    /// into a single download — without this, racing tasks would both
    /// truncate the destination archive and interleave writes, potentially
    /// renaming corrupt bytes into place. The kpar is downloaded
    /// directly to the destination path, so `is_downloaded_and_verified`
    /// must be checked before reading it.
    /// Errors aren't cached, so a transient failure is retryable.
    /// If this is initialized, the archive is downloaded and verified
    /// against `expected_sha256_hex`/`expected_size` if present.
    downloaded_verified: tokio::sync::OnceCell<()>,
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
    #[error("kpar at `{url}` has sha256 `{computed}` but the expected digest was `{expected}`")]
    DigestMismatch {
        url: Box<str>,
        expected: String,
        computed: String,
    },
    #[error("kpar at `{url}` has size {actual} bytes but the expected size was {expected} bytes")]
    SizeMismatch {
        url: Box<str>,
        expected: u64,
        actual: u64,
    },
    #[error("expected kpar size for `{url}` must be non-zero")]
    ZeroExpectedSize { url: Box<str> },
}

impl From<FsIoError> for ReqwestKparDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<Policy: HTTPAuthentication> ReqwestKparDownloadedProject<Policy> {
    pub fn new(
        url: reqwest::Url,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        expected_sha256_hex: Option<String>,
        expected_size: Option<u64>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        let expected_size = match expected_size {
            Some(size) => Some(NonZeroU64::new(size).ok_or_else(|| {
                ReqwestKparDownloadedError::ZeroExpectedSize {
                    url: url.as_str().into(),
                }
            })?),
            None => None,
        };

        Ok(Self {
            url,
            inner: LocalKParProject::new_temporary()?,
            client,
            auth_policy,
            expected_sha256_hex,
            expected_size,
            downloaded_verified: tokio::sync::OnceCell::new(),
        })
    }

    pub fn new_guess_root<S: AsRef<str>>(
        url: S,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        expected_sha256_hex: Option<String>,
        expected_size: Option<u64>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        Self::new(
            reqwest::Url::parse(url.as_ref())
                .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?,
            client,
            auth_policy,
            expected_sha256_hex,
            expected_size,
        )
    }

    /// True iff the archive is on disk and has been successfully
    /// verified against expected hex and length (if present)
    pub fn is_downloaded_and_verified(&self) -> bool {
        self.downloaded_verified.initialized()
    }

    /// Ensure the archive is on disk and verify the digest if known
    pub async fn ensure_downloaded_verified(&self) -> Result<(), ReqwestKparDownloadedError> {
        self.downloaded_verified
            .get_or_try_init(|| self.perform_download())
            .await?;
        Ok(())
    }

    /// Download the archive. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share the single in-flight attempt and a returned `Err` leaves
    /// the cell uninitialized (retries succeed).
    ///
    /// Downloads directly to the final path. Callers must go through
    /// `ensure_downloaded_verified` before reading so the `OnceCell` has
    /// observed a successful download and any configured verification.
    async fn perform_download(&self) -> Result<(), ReqwestKparDownloadedError> {
        use futures::StreamExt as _;

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

        if let (Some(actual), Some(expected)) = (resp.content_length(), self.expected_size)
            && actual != expected.get()
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: expected.get(),
                actual,
            });
        }

        let mut bytes_stream = resp.bytes_stream();
        let mut hasher = self.expected_sha256_hex.as_deref().map(|_| Sha256::new());
        let mut written = 0_u64;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(ReqwestKparDownloadedError::Reqwest)?;
            written += bytes.len() as u64;
            if let Some(expected) = self.expected_size
                && written > expected.get()
            {
                return Err(ReqwestKparDownloadedError::SizeMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.get(),
                    actual: written,
                });
            }
            if let Some(h) = hasher.as_mut() {
                h.update(&bytes);
            }
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.clone(), e))?;
        }

        if let Some(expected) = self.expected_size
            && written != expected.get()
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: expected.get(),
                actual: written,
            });
        }

        file.sync_all()
            .map_err(|e| FsIoError::WriteFile(self.inner.archive_path.clone(), e))?;

        if let Some(expected) = self.expected_size {
            debug_assert_eq!(expected.get(), self.inner.file_size().unwrap());
        }

        if let (Some(h), Some(expected)) = (hasher, self.expected_sha256_hex.as_deref()) {
            let computed = format!("{:x}", h.finalize());
            if computed == expected {
                return Ok(());
            } else {
                return Err(ReqwestKparDownloadedError::DigestMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.to_owned(),
                    computed,
                });
            }
        }

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
        self.ensure_downloaded_verified().await?;
        Ok(self.inner.get_project()?)
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        self.ensure_downloaded_verified().await?;
        Ok(self.inner.get_info()?)
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        self.ensure_downloaded_verified().await?;
        Ok(self.inner.get_meta()?)
    }

    type SourceReader<'a>
        = AsAsyncRead<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.ensure_downloaded_verified().await?;

        Ok(AsAsyncRead {
            inner: self.inner.read_source(path)?,
        })
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let src = if let (Some(index_kpar_size), Some(index_kpar_digest)) =
            (self.expected_size, self.expected_sha256_hex.as_ref())
        {
            Source::IndexKpar {
                index_kpar: self.url.to_string(),
                index_kpar_size: index_kpar_size.get(),
                index_kpar_digest: index_kpar_digest.clone(),
            }
        } else {
            Source::RemoteKpar {
                remote_kpar: self.url.to_string(),
                remote_kpar_size: self.inner.file_size().ok(),
            }
        };
        Ok(vec![src])
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        self.inner.is_definitely_invalid()
    }
}

#[cfg(test)]
#[path = "./reqwest_kpar_download_tests.rs"]
mod tests;
