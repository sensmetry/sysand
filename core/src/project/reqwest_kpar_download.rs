// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    error::Error,
    io::{self, Write as _},
    num::NonZeroU64,
    pin::Pin,
    sync::Arc,
};

use camino_tempfile::tempdir;
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
        local_kpar::{LocalKParError, LocalKParProject, LocalKParProjectRaw},
    },
    resolve::net_utils::kpar_get_request,
    utils::lowercase_hex,
};

use super::{
    ProjectChecksum,
    local_kpar::KparInnerPath,
    utils::{FsIoError, wrapfs},
};

#[derive(Debug)]
pub struct KparMeta {
    pub size_bytes: NonZeroU64,
    pub sha256_hex: String,
}

/// Project stored at a remote URL such as https://www.example.com/project.kpar.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// Downloads the full archive to a temporary directory and then accesses it using
/// `LocalKParProject`.
#[derive(Debug)]
pub struct ReqwestRemoteKparDownloadedProject<Policy> {
    // inner: LocalKParProjectRaw,
    // tmp_dir: Utf8TempDir,
    // archive_path: Utf8PathBuf,
    url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    /// Optionally contains:
    ///
    /// - expected sha256 hex digest for callers that need this
    ///   instance to enforce archive verification before exposing project
    ///   contents (e.g. lockfile-driven `sync`).
    /// - expected archive byte length. Index-backed kpars carry this
    ///   in the lockfile / versions index; enforce it while streaming so a
    ///   malicious server cannot exhaust disk before the digest check fails.
    expected: Option<KparMeta>,
    /// Fans concurrent `ensure_downloaded*` calls on the same instance
    /// into a single download — without this, racing tasks would both
    /// truncate the destination archive and interleave writes.
    ///
    /// The kpar is downloaded directly to the destination path, so
    /// `is_downloaded_and_verified` must be checked before reading it.
    /// Errors aren't cached, so a transient failure is retryable.
    ///
    /// If this is initialized, the archive has been downloaded and verified
    /// against `expected` if present (then this will be empty), and
    /// otherwise contains actual downloaded KPAR metadata.
    downloaded_verified: tokio::sync::OnceCell<(LocalKParProjectRaw, Option<KparMeta>)>,
}

#[derive(Debug)]
pub struct ReqwestIndexKparDownloadedProject<Policy> {
    // inner: LocalKParProjectRaw,
    url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    /// Expected sha256 hex digest for callers that need this
    /// instance to enforce archive verification before exposing project
    /// contents (e.g. lockfile-driven `sync`).
    expected_kpar_sha256: String,
    /// Expected archive byte length. Index-backed kpars carry this
    /// in the lockfile / versions index; enforce it while streaming so a
    /// malicious server cannot exhaust disk before the digest check fails.
    expected_size: NonZeroU64,
    /// Fans concurrent `ensure_downloaded*` calls on the same instance
    /// into a single download — without this, racing tasks would both
    /// truncate the destination archive and interleave writes.
    ///
    /// The kpar is downloaded directly to the destination path, so
    /// `is_downloaded_and_verified` must be checked before reading it.
    /// Errors aren't cached, so a transient failure is retryable.
    /// If this is initialized, the archive has been downloaded and verified
    /// against `expected_sha256_hex`/`expected_size` if present.
    downloaded_verified: tokio::sync::OnceCell<LocalKParProjectRaw>,
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
    // TODO: ensure that error chain is printed, then remove cause
    // details from this message
    #[error("error making an HTTP request:\n{0:#?}")]
    Reqwest(#[from] reqwest::Error),
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
    #[error("kpar at `{url}` is an empty file")]
    EmptyKpar { url: Box<str> },
}

impl From<FsIoError> for ReqwestKparDownloadedError {
    fn from(v: FsIoError) -> Self {
        Self::Io(Box::new(v))
    }
}

impl<Policy: HTTPAuthentication> ReqwestRemoteKparDownloadedProject<Policy> {
    // TODO: indicate to `inner` that it should not guess root
    // Also decide whether to take URL or str in all constructors here, and
    // do so consistently for all project types
    pub fn new_guess_root<S: AsRef<str>>(
        url: S,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        expected: Option<KparMeta>,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        let url = reqwest::Url::parse(url.as_ref())
            .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?;
        Ok(Self {
            url,
            client,
            auth_policy,
            expected,
            downloaded_verified: tokio::sync::OnceCell::new(),
        })
    }

    /// True iff the archive is on disk and has been successfully
    /// verified against expected hex and length (if present)
    pub fn is_downloaded_and_verified(&self) -> bool {
        self.downloaded_verified.initialized()
    }

    /// Ensure the archive is on disk and verify the digest if known.
    /// THe digest is returned iff it's not known.
    pub async fn ensure_downloaded_verified(
        &self,
    ) -> Result<&(LocalKParProjectRaw, Option<KparMeta>), ReqwestKparDownloadedError> {
        self.downloaded_verified
            .get_or_try_init(|| self.perform_download())
            .await
    }

    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    /// Download the archive. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share the single in-flight attempt and a returned `Err` leaves
    /// the cell uninitialized (retries succeed).
    ///
    /// Downloads directly to the final path. Callers must go through
    /// `ensure_downloaded_verified` before reading so the `OnceCell` has
    /// observed a successful download and any configured verification.
    async fn perform_download(
        &self,
    ) -> Result<(LocalKParProjectRaw, Option<KparMeta>), ReqwestKparDownloadedError> {
        use futures::StreamExt as _;

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

        if let (Some(actual), Some(expected)) = (resp.content_length(), &self.expected)
            && actual != expected.size_bytes.get()
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: expected.size_bytes.get(),
                actual,
            });
        }

        let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;
        let archive_path = tmp_dir.path().join("project.kpar");
        let mut file = wrapfs::File::create(&archive_path)?;
        let mut bytes_stream = resp.bytes_stream();
        let mut hasher = Sha256::new();
        let mut written = 0_u64;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(ReqwestKparDownloadedError::Reqwest)?;
            written += bytes.len() as u64;
            if let Some(expected) = &self.expected
                && written > expected.size_bytes.get()
            {
                return Err(ReqwestKparDownloadedError::SizeMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.size_bytes.get(),
                    actual: written,
                });
            }
            hasher.update(&bytes);
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;
        }
        let written = if let Some(w) = NonZeroU64::new(written) {
            w
        } else {
            return Err(ReqwestKparDownloadedError::EmptyKpar {
                url: self.url.as_str().into(),
            });
        };

        if let Some(expected) = &self.expected
            && written != expected.size_bytes
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: expected.size_bytes.get(),
                actual: written.get(),
            });
        }

        file.sync_all()
            .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;

        let computed_hash = lowercase_hex(hasher.finalize());
        if let Some(expected) = &self.expected {
            debug_assert_eq!(
                expected.size_bytes.get(),
                wrapfs::metadata(&archive_path).unwrap().len()
            );

            if computed_hash == expected.sha256_hex {
                let inner =
                    LocalKParProjectRaw::new_tempdir(tmp_dir, archive_path, KparInnerPath::Guess)?;
                Ok((inner, None))
            } else {
                Err(ReqwestKparDownloadedError::DigestMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.sha256_hex.to_owned(),
                    computed: computed_hash,
                })
            }
        } else {
            let inner =
                LocalKParProjectRaw::new_tempdir(tmp_dir, archive_path, KparInnerPath::Guess)?;
            let meta = KparMeta {
                size_bytes: written,
                sha256_hex: computed_hash,
            };
            Ok((inner, Some(meta)))
        }
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

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestRemoteKparDownloadedProject<Policy> {
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
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(inner.get_project()?),
            Err(e) => Err(e),
        }
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(inner.get_info()?),
            Err(e) => Err(e),
        }
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(inner.get_meta()?),
            Err(e) => Err(e),
        }
    }

    type SourceReader<'a>
        = AsAsyncRead<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(AsAsyncRead {
                inner: inner.read_source(path)?,
            }),
            Err(e) => Err(e),
        }
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let (kpar_size, kpar_digest) = if let Some(expected) = &self.expected {
            (expected.size_bytes, expected.sha256_hex.to_owned())
        } else {
            // If expected is not present, download populates the cell with actual
            let (_, maybe_meta) = self.ensure_downloaded_verified().await?;
            let actual_meta = maybe_meta.as_ref().unwrap();
            (actual_meta.size_bytes, actual_meta.sha256_hex.to_owned())
        };
        Ok(vec![Source::RemoteKpar {
            remote_kpar: self.url.to_string(),
            kpar_size,
            kpar_digest,
        }])
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        // FIXME: error should be returned
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => inner.is_definitely_invalid(),
            Err(e) => {
                // TODO: generalize `format_sources()` to logging
                log::debug!("error downloading/reading a kpar: {e}");
                let mut error: &dyn Error = &e;
                while let Some(source) = error.source() {
                    log::debug!("  caused by: {source}");
                    error = source;
                }

                false
            }
        }
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((_, meta)) => {
                let meta = match meta {
                    Some(m) => m,
                    None => self.expected.as_ref().unwrap(),
                };
                Ok(ProjectChecksum::Kpar(meta.sha256_hex.to_owned()))
            }
            Err(e) => Err(e),
        }
    }
}

impl<Policy: HTTPAuthentication> ReqwestIndexKparDownloadedProject<Policy> {
    pub fn new(
        url: reqwest::Url,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        expected_size: NonZeroU64,
        expected_kpar_sha256: String,
    ) -> Result<Self, ReqwestKparDownloadedError> {
        let url = reqwest::Url::parse(url.as_ref())
            .map_err(|e| ReqwestKparDownloadedError::ParseUrl(url.as_ref().into(), e))?;
        Ok(Self {
            url,
            client,
            auth_policy,
            expected_kpar_sha256,
            expected_size,
            downloaded_verified: tokio::sync::OnceCell::new(),
        })
    }

    /// True iff the archive is on disk and has been successfully
    /// verified against expected hex and length (if present)
    pub fn is_downloaded_and_verified(&self) -> bool {
        self.downloaded_verified.initialized()
    }

    /// Ensure the archive is on disk and verify the digest if known
    pub async fn ensure_downloaded_verified(
        &self,
    ) -> Result<&LocalKParProjectRaw, ReqwestKparDownloadedError> {
        self.downloaded_verified
            .get_or_try_init(|| self.perform_download())
            .await
    }

    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }

    /// Download the archive. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share the single in-flight attempt and a returned `Err` leaves
    /// the cell uninitialized (retries succeed).
    ///
    /// Downloads directly to the final path. Callers must go through
    /// `ensure_downloaded_verified` before reading so the `OnceCell` has
    /// observed a successful download and verification.
    async fn perform_download(&self) -> Result<LocalKParProjectRaw, ReqwestKparDownloadedError> {
        use futures::StreamExt as _;

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

        if let Some(actual) = resp.content_length()
            && actual != self.expected_size.get()
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: self.expected_size.get(),
                actual,
            });
        }

        let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;
        let archive_path = tmp_dir.path().join("project.kpar");
        let mut file = wrapfs::File::create(&archive_path)?;
        let mut bytes_stream = resp.bytes_stream();
        let mut hasher = Sha256::new();
        let mut written = 0_u64;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(ReqwestKparDownloadedError::Reqwest)?;
            written += bytes.len() as u64;
            if written > self.expected_size.get() {
                return Err(ReqwestKparDownloadedError::SizeMismatch {
                    url: self.url.as_str().into(),
                    expected: self.expected_size.get(),
                    actual: written,
                });
            }
            hasher.update(&bytes);
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;
        }
        let written = if let Some(w) = NonZeroU64::new(written) {
            w
        } else {
            return Err(ReqwestKparDownloadedError::EmptyKpar {
                url: self.url.as_str().into(),
            });
        };

        if written != self.expected_size {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected: self.expected_size.get(),
                actual: written.get(),
            });
        }

        file.sync_all()
            .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;

        let computed_hash = lowercase_hex(hasher.finalize());
        debug_assert_eq!(
            self.expected_size.get(),
            wrapfs::metadata(&archive_path).unwrap().len()
        );

        debug_assert_eq!(
            self.expected_size.get(),
            wrapfs::metadata(&archive_path).unwrap().len()
        );

        if computed_hash == self.expected_kpar_sha256 {
            let inner =
                LocalKParProjectRaw::new_tempdir(tmp_dir, archive_path, KparInnerPath::Guess)?;
            Ok(inner)
        } else {
            Err(ReqwestKparDownloadedError::DigestMismatch {
                url: self.url.as_str().into(),
                expected: self.expected_kpar_sha256.to_owned(),
                computed: computed_hash,
            })
        }
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestIndexKparDownloadedProject<Policy> {
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
        Ok(self.ensure_downloaded_verified().await?.get_project()?)
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(self.ensure_downloaded_verified().await?.get_info()?)
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(self.ensure_downloaded_verified().await?.get_meta()?)
    }

    type SourceReader<'a>
        = AsAsyncRead<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok(inner) => Ok(AsAsyncRead {
                inner: inner.read_source(path)?,
            }),
            Err(e) => Err(e),
        }
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::IndexKpar {
            index_kpar: self.url.to_string(),
            kpar_size: self.expected_size,
            kpar_digest: self.expected_kpar_sha256.clone(),
        }])
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        // FIXME: error should be returned; does it make sense to download the project
        // here, as this is supposed to be a quick check
        match self.ensure_downloaded_verified().await {
            Ok(inner) => inner.is_definitely_invalid(),
            Err(e) => {
                // TODO: generalize `format_sources()` to logging
                log::debug!("error downloading/reading a kpar: {e}");
                let mut error: &dyn Error = &e;
                while let Some(source) = error.source() {
                    log::debug!("  caused by: {source}");
                    error = source;
                }

                false
            }
        }
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok(_) => Ok(ProjectChecksum::Kpar(self.expected_kpar_sha256.clone())),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
#[path = "./reqwest_kpar_download_tests.rs"]
mod tests;
