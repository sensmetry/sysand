// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{self, Read as _, Write as _},
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
    /// Optional expected sha256 hex digest for callers that need this
    /// instance to enforce archive verification before exposing project
    /// contents (e.g. lockfile-driven `sync`).
    expected_sha256_hex: Option<String>,
    /// Optional expected archive byte length. Index-backed kpars carry this
    /// in the lockfile / versions index; enforce it while streaming so a
    /// malicious server cannot exhaust disk before the digest check fails.
    expected_size: Option<u64>,
    /// Fans concurrent `ensure_downloaded*` calls on the same instance
    /// into a single download — without this, racing tasks would both
    /// truncate the staging file and interleave writes, potentially
    /// renaming corrupt bytes into place. Set once bytes are at
    /// `archive_path`, regardless of whether the download was verified.
    /// Errors aren't cached, so a transient failure is retryable.
    downloaded: tokio::sync::OnceCell<()>,
    /// The sha256 hex digest the on-disk archive has been verified
    /// against. Populated by the first successful
    /// `ensure_downloaded_verified` call. Any later
    /// `ensure_downloaded_verified(d')` with `d' != d` hard-fails —
    /// archive bytes are write-once per instance and the advertised
    /// digest is a stable identifier, so divergent expected digests
    /// cannot both be valid.
    verified: tokio::sync::OnceCell<String>,
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
            expected_sha256_hex: None,
            expected_size: None,
            downloaded: tokio::sync::OnceCell::new(),
            verified: tokio::sync::OnceCell::new(),
        })
    }

    pub fn with_expected_sha256_hex<S: AsRef<str>>(mut self, expected_sha256_hex: S) -> Self {
        self.expected_sha256_hex = Some(expected_sha256_hex.as_ref().to_owned());
        self
    }

    pub fn with_expected_size(mut self, expected_size: u64) -> Self {
        self.expected_size = Some(expected_size);
        self
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

    /// True iff the archive is on disk at its final path. The archive
    /// is only ever renamed into place after a successful staging
    /// write, so `is_file` is sufficient. Note that this returns `true`
    /// for bytes that have *not* been verified against a digest —
    /// callers that need verification should rely on
    /// [`Self::ensure_downloaded_verified`] instead.
    pub fn is_downloaded(&self) -> bool {
        self.inner.archive_path.is_file()
    }

    async fn ensure_ready(&self) -> Result<(), ReqwestKparDownloadedError> {
        if let Some(expected_sha256_hex) = self.expected_sha256_hex.as_deref() {
            self.ensure_downloaded_verified(expected_sha256_hex).await
        } else {
            self.ensure_downloaded().await
        }
    }

    /// True iff the on-disk archive has been verified against an
    /// advertised sha256 hex digest by a prior successful
    /// [`Self::ensure_downloaded_verified`] call. Distinct from
    /// [`Self::is_downloaded`]: an unverified
    /// [`Self::ensure_downloaded`] renames bytes into place without
    /// ever consulting a digest, so a caller that needs to know the
    /// bytes are known-good (e.g. before comparing a locally-computed
    /// canonical digest against the advertised one) must gate on
    /// `is_verified`, not `is_downloaded`.
    pub fn is_verified(&self) -> bool {
        self.verified.get().is_some()
    }

    /// Ensure the archive is on disk without verifying its digest. Use
    /// [`Self::ensure_downloaded_verified`] when the caller has a sha256 hex
    /// digest to check against.
    pub async fn ensure_downloaded(&self) -> Result<(), ReqwestKparDownloadedError> {
        self.downloaded
            .get_or_try_init(|| self.perform_download(None, self.expected_size))
            .await?;
        Ok(())
    }

    /// Ensure the archive is on disk *and* its sha256 matched
    /// `expected_sha256_hex`. The hex is compared lowercase; callers
    /// obtain it from the pre-validated `Sha256HexDigest` produced during
    /// `versions.json` ingest.
    ///
    /// If a previous unverified `ensure_downloaded()` already populated
    /// `archive_path`, this method re-hashes the local file rather than
    /// skipping verification — so an instance cannot silently serve
    /// unverified bytes to a later caller that asked for verification.
    /// On second and subsequent verified calls the stored digest is
    /// compared directly, so the rehash happens at most once per
    /// instance.
    pub async fn ensure_downloaded_verified(
        &self,
        expected_sha256_hex: &str,
    ) -> Result<(), ReqwestKparDownloadedError> {
        // Fast-path: we've already verified against *some* digest. If
        // it's the same one, return Ok. If it's different, hard-fail
        // without touching the file — the first verified digest is the
        // authoritative one.
        if let Some(prev) = self.verified.get() {
            return if prev == expected_sha256_hex {
                Ok(())
            } else {
                Err(ReqwestKparDownloadedError::DigestMismatch {
                    url: self.url.as_str().into(),
                    expected: expected_sha256_hex.to_owned(),
                    computed: prev.clone(),
                })
            };
        }

        // Populate `verified` with the digest of whatever ends up on
        // disk. Racing verified callers fan in to a single pass here.
        let stored = self
            .verified
            .get_or_try_init(|| async {
                if self.downloaded.get().is_some() {
                    // A prior *unverified* download already renamed
                    // bytes into place. Re-hash the on-disk file to
                    // publish the authoritative digest.
                    self.verify_archive_size()?;
                    return hash_archive_sha256_hex(&self.inner.archive_path);
                }
                // No prior download. Download with inline verification
                // so that a mismatched body never gets renamed into
                // `archive_path` — `perform_download(Some(expected))`
                // aborts before the atomic rename on digest mismatch,
                // leaving `downloaded` uninitialized (retry-safe).
                self.downloaded
                    .get_or_try_init(|| {
                        self.perform_download(Some(expected_sha256_hex), self.expected_size)
                    })
                    .await?;
                // Either our download succeeded (so `expected` matches
                // the bytes on disk) or a concurrent caller raced past
                // our `downloaded.get().is_some()` check and initialized
                // the cell — in that case the bytes on disk may be
                // whatever that caller downloaded. Hash the file to
                // publish the authoritative digest either way.
                hash_archive_sha256_hex(&self.inner.archive_path)
            })
            .await?;

        if stored != expected_sha256_hex {
            return Err(ReqwestKparDownloadedError::DigestMismatch {
                url: self.url.as_str().into(),
                expected: expected_sha256_hex.to_owned(),
                computed: stored.clone(),
            });
        }
        Ok(())
    }

    /// Perform the one-shot download. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share the single in-flight attempt and a returned `Err` leaves
    /// the cell uninitialized (retries succeed).
    ///
    /// Downloads to a sibling staging path and atomic-renames on success.
    /// `archive_path.is_file()` is then the "verified" sentinel: a
    /// failed verification never promotes bytes to `final_path`, so a
    /// retry can never serve tampered bytes. Partial staging files left
    /// behind by errors are reclaimed when the owning tempdir drops;
    /// retries within the same instance truncate via `File::create`.
    async fn perform_download(
        &self,
        expected_digest: Option<&str>,
        expected_size: Option<u64>,
    ) -> Result<(), ReqwestKparDownloadedError> {
        use futures::StreamExt as _;

        let final_path = &self.inner.archive_path;
        let staging_path = staging_path_for(final_path);
        let mut file = wrapfs::File::create(&staging_path)?;

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

        if let (Some(actual), Some(expected)) = (resp.content_length(), expected_size)
            && actual != expected
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected,
                actual,
            });
        }

        let mut bytes_stream = resp.bytes_stream();
        let mut hasher = expected_digest.map(|_| Sha256::new());
        let mut written = 0_u64;

        while let Some(bytes) = bytes_stream.next().await {
            let bytes = bytes.map_err(ReqwestKparDownloadedError::Reqwest)?;
            written = written.checked_add(bytes.len() as u64).ok_or_else(|| {
                ReqwestKparDownloadedError::SizeMismatch {
                    url: self.url.as_str().into(),
                    expected: expected_size.unwrap_or(u64::MAX),
                    actual: u64::MAX,
                }
            })?;
            if let Some(expected) = expected_size
                && written > expected
            {
                return Err(ReqwestKparDownloadedError::SizeMismatch {
                    url: self.url.as_str().into(),
                    expected,
                    actual: written,
                });
            }
            if let Some(h) = hasher.as_mut() {
                h.update(&bytes);
            }
            file.write_all(&bytes)
                .map_err(|e| FsIoError::WriteFile(staging_path.clone(), e))?;
        }

        if let Some(expected) = expected_size
            && written != expected
        {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected,
                actual: written,
            });
        }

        // Verify before any sync_all/rename so a mismatched archive never
        // gets durably installed at `final_path`.
        if let (Some(h), Some(expected)) = (hasher, expected_digest) {
            let computed = format!("{:x}", h.finalize());
            if computed != expected {
                return Err(ReqwestKparDownloadedError::DigestMismatch {
                    url: self.url.as_str().into(),
                    expected: expected.to_owned(),
                    computed,
                });
            }
        }

        file.sync_all()
            .map_err(|e| FsIoError::WriteFile(staging_path.clone(), e))?;
        drop(file);

        crate::env::local_directory::utils::move_fs_item(&staging_path, final_path)?;

        Ok(())
    }

    fn verify_archive_size(&self) -> Result<(), ReqwestKparDownloadedError> {
        let Some(expected) = self.expected_size else {
            return Ok(());
        };
        let actual = self.inner.file_size()?;
        if actual != expected {
            return Err(ReqwestKparDownloadedError::SizeMismatch {
                url: self.url.as_str().into(),
                expected,
                actual,
            });
        }
        Ok(())
    }
}

/// Hash a local archive file, returning the lowercase sha256 hex
/// digest. Sync I/O, matching the rest of this module's file-access
/// conventions.
fn hash_archive_sha256_hex(
    archive_path: &camino::Utf8Path,
) -> Result<String, ReqwestKparDownloadedError> {
    let mut file = wrapfs::File::open(archive_path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| FsIoError::ReadFile(archive_path.into(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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
        self.ensure_ready().await?;
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
        self.ensure_ready().await?;

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
