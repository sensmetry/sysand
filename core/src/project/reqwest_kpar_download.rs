// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{io::Write as _, num::NonZeroU64, sync::Arc};

use camino_tempfile::tempdir;
use fluent_uri::Iri;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    project::{
        AsAsyncReader, KparMeta, ProjectDefinition, ProjectRead, ProjectReadAsync,
        local_kpar::{LocalKParError, LocalKParProject, LocalKParProjectRaw},
    },
    redact::{HttpFailure, ResponseUrl, redact_userinfo},
    resolve::net_utils::kpar_get_request,
    utils::{format_err, lowercase_hex},
};

use super::{
    ProjectChecksum,
    local_kpar::KparInnerPath,
    utils::{FsIoError, wrapfs},
};

/// Project stored at a remote URL such as <https://www.example.com/project.kpar>.
/// The URL is expected to resolve to a kpar-archive (ZIP-file) (at least) if
/// requested with CONTENT-TYPE(s) application/zip, application/x-zip-compressed.
///
/// See `LocalKParProject` for additional details on the format.
///
/// Downloads the full archive to a temporary directory and then accesses it using
/// `LocalKParProject`.
#[derive(Debug)]
pub struct ReqwestRemoteKparDownloadedProject<Policy> {
    url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    /// Expected size and sha256 digest, for callers that need archive
    /// verification before project contents are exposed (e.g.
    /// lockfile-driven `sync`). The size is enforced while streaming so a
    /// malicious server cannot exhaust disk before the digest check fails.
    expected: Option<KparMeta>,
    /// Fans concurrent `ensure_downloaded*` calls into a single download.
    /// Errors are not cached, so a transient failure is retryable.
    ///
    /// Initialized only once the archive has been downloaded and verified
    /// against `expected`, if present. The metadata is `None` in that case
    /// and otherwise describes the downloaded archive.
    downloaded_verified: tokio::sync::OnceCell<(LocalKParProjectRaw, Option<KparMeta>)>,
}

#[derive(Debug)]
pub struct ReqwestIndexKparDownloadedProject<Policy> {
    url: reqwest::Url,
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    /// Expected size and sha256 digest, verified before project contents
    /// are exposed. The size is enforced while streaming so a malicious
    /// server cannot exhaust disk before the digest check fails.
    expected: KparMeta,
    /// Fans concurrent `ensure_downloaded*` calls into a single download.
    /// Errors are not cached, so a transient failure is retryable.
    ///
    /// Initialized only once the archive has been downloaded and verified
    /// against `expected`.
    downloaded_verified: tokio::sync::OnceCell<LocalKParProjectRaw>,
}

// TODO: reduce size of errors here and elsewhere
#[derive(Error, Debug)]
pub enum ReqwestKparDownloadedError {
    #[error(transparent)]
    Http(#[from] HttpFailure),
    #[error("failed to parse URL `{0}`")]
    ParseUrl(Box<str>, #[source] url::ParseError),
    #[error(transparent)]
    KPar(#[from] LocalKParError),
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
    #[error("kpar at {url} has sha256 `{computed}` but the expected digest was `{expected}`")]
    DigestMismatch {
        url: ResponseUrl,
        expected: String,
        computed: String,
    },
    #[error("kpar at {url} has size {actual} bytes but the expected size was {expected} bytes")]
    SizeMismatch {
        url: ResponseUrl,
        expected: u64,
        actual: u64,
    },
    #[error("kpar at {url} is an empty file")]
    EmptyKpar { url: ResponseUrl },
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
            .map_err(|e| ReqwestKparDownloadedError::ParseUrl(redact_userinfo(url.as_ref()), e))?;
        Ok(Self {
            url,
            client,
            auth_policy,
            expected,
            downloaded_verified: tokio::sync::OnceCell::new(),
        })
    }

    /// True iff the archive has been downloaded and verified against the
    /// expected digest and size (if present).
    pub fn is_downloaded_and_verified(&self) -> bool {
        self.downloaded_verified.initialized()
    }

    /// Ensure the archive is on disk and verify the digest if known. The
    /// archive's metadata is returned iff the digest was not known.
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

    /// Download the archive into a fresh temp dir. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share one attempt and an `Err` leaves the cell uninitialized.
    async fn perform_download(
        &self,
    ) -> Result<(LocalKParProjectRaw, Option<KparMeta>), ReqwestKparDownloadedError> {
        let (inner, meta) = download(
            &self.url,
            &self.client,
            &*self.auth_policy,
            self.expected.as_ref(),
        )
        .await?;
        Ok((inner, self.expected.is_none().then_some(meta)))
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestRemoteKparDownloadedProject<Policy> {
    type Error = ReqwestKparDownloadedError;

    async fn get_project_async(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(inner.get_project()?),
            Err(e) => Err(e),
        }
    }

    type SourceReader<'a>
        = AsAsyncReader<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => Ok(AsAsyncReader::new(inner.read_source(path)?)),
            Err(e) => Err(e),
        }
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let (kpar_size, kpar_digest) = if let Some(expected) = &self.expected {
            (expected.size_bytes, expected.sha256_hex.clone())
        } else {
            // Without `expected`, the download fills in the actual metadata
            let (_, maybe_meta) = self.ensure_downloaded_verified().await?;
            let actual_meta = maybe_meta.as_ref().unwrap();
            (actual_meta.size_bytes, actual_meta.sha256_hex.clone())
        };
        Ok(vec![Source::RemoteKpar {
            // Cannot panic: the URL was created from an IRI
            remote_kpar: Iri::parse(self.url.to_string()).unwrap(),
            kpar_size,
            kpar_digest,
        }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A URL names one archive.
        false
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        // FIXME: error should be returned
        match self.ensure_downloaded_verified().await {
            Ok((inner, _)) => inner.is_definitely_invalid(),
            Err(e) => {
                log::debug!("error downloading/reading a kpar: {}", format_err(&e));
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
                Ok(ProjectChecksum::Kpar(meta.sha256_hex.clone()))
            }
            Err(e) => Err(e),
        }
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
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
            .map_err(|e| ReqwestKparDownloadedError::ParseUrl(redact_userinfo(url.as_ref()), e))?;
        Ok(Self {
            url,
            client,
            auth_policy,
            expected: KparMeta {
                size_bytes: expected_size,
                sha256_hex: expected_kpar_sha256,
            },
            downloaded_verified: tokio::sync::OnceCell::new(),
        })
    }

    /// True iff the archive has been downloaded and verified against the
    /// expected digest and size (if present).
    pub fn is_downloaded_and_verified(&self) -> bool {
        self.downloaded_verified.initialized()
    }

    /// Ensure the archive is on disk and verified
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

    /// Download the archive into a fresh temp dir. Invoked through
    /// [`tokio::sync::OnceCell::get_or_try_init`], so concurrent callers
    /// share one attempt and an `Err` leaves the cell uninitialized.
    async fn perform_download(&self) -> Result<LocalKParProjectRaw, ReqwestKparDownloadedError> {
        let (inner, _) = download(
            &self.url,
            &self.client,
            &*self.auth_policy,
            Some(&self.expected),
        )
        .await?;
        Ok(inner)
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestIndexKparDownloadedProject<Policy> {
    type Error = ReqwestKparDownloadedError;

    async fn get_project_async(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        Ok(self.ensure_downloaded_verified().await?.get_project()?)
    }

    type SourceReader<'a>
        = AsAsyncReader<<LocalKParProject as ProjectRead>::SourceReader<'a>>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok(inner) => Ok(AsAsyncReader::new(inner.read_source(path)?)),
            Err(e) => Err(e),
        }
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::IndexKpar {
            // Cannot panic: the URL was created from an IRI
            index_kpar: Iri::parse(self.url.to_string()).unwrap(),
            kpar_size: self.expected.size_bytes,
            kpar_digest: self.expected.sha256_hex.clone(),
        }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // An index advertises every version of a project it holds.
        true
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        // FIXME: error should be returned; does it make sense to download the project
        // here (this is supposed to be a quick check)?
        match self.ensure_downloaded_verified().await {
            Ok(inner) => inner.is_definitely_invalid(),
            Err(e) => {
                log::debug!("error downloading/reading a kpar: {}", format_err(&e));
                false
            }
        }
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        match self.ensure_downloaded_verified().await {
            Ok(_) => Ok(ProjectChecksum::Kpar(self.expected.sha256_hex.clone())),
            Err(e) => Err(e),
        }
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
}

/// Download the archive at `url` into a fresh temp dir, enforcing the
/// `expected` size while streaming and checking its digest at the end.
/// Returns the archive with its actual size and digest.
async fn download<Policy: HTTPAuthentication>(
    url: &reqwest::Url,
    client: &reqwest_middleware::ClientWithMiddleware,
    auth_policy: &Policy,
    expected: Option<&KparMeta>,
) -> Result<(LocalKParProjectRaw, KparMeta), ReqwestKparDownloadedError> {
    use futures::StreamExt as _;

    let resp = auth_policy
        .with_authentication(client, &kpar_get_request(url.clone()))
        .await
        .map_err(|source| HttpFailure::Request {
            url: url.into(),
            source: source.into(),
        })?;
    let response_url = ResponseUrl::new(url, resp.url());

    if !resp.status().is_success() {
        return Err(HttpFailure::BadHttpStatus {
            url: response_url,
            status: resp.status(),
        }
        .into());
    }

    let size_mismatch = |actual| ReqwestKparDownloadedError::SizeMismatch {
        url: response_url.clone(),
        expected: expected.map_or(0, |expected| expected.size_bytes.get()),
        actual,
    };
    let too_big = |actual| expected.is_some_and(|expected| actual > expected.size_bytes.get());
    let differs = |actual| expected.is_some_and(|expected| actual != expected.size_bytes.get());

    if let Some(actual) = resp.content_length()
        && differs(actual)
    {
        return Err(size_mismatch(actual));
    }

    let tmp_dir = tempdir().map_err(FsIoError::MkTempDir)?;
    let archive_path = tmp_dir.path().join("project.kpar");
    let mut file = wrapfs::File::create(&archive_path)?;
    let mut bytes_stream = resp.bytes_stream();
    let mut hasher = Sha256::new();
    let mut written = 0u64;

    while let Some(bytes) = bytes_stream.next().await {
        let bytes = bytes.map_err(|source| HttpFailure::Body {
            url: response_url.clone(),
            source: source.into(),
        })?;
        written += bytes.len() as u64;
        if too_big(written) {
            return Err(size_mismatch(written));
        }
        hasher.update(&bytes);
        file.write_all(&bytes)
            .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;
    }
    let Some(written) = NonZeroU64::new(written) else {
        return Err(ReqwestKparDownloadedError::EmptyKpar { url: response_url });
    };
    if differs(written.get()) {
        return Err(size_mismatch(written.get()));
    }

    file.sync_all()
        .map_err(|e| FsIoError::WriteFile(archive_path.clone(), e))?;

    let computed_hash = lowercase_hex(hasher.finalize());
    if let Some(expected) = expected
        && computed_hash != expected.sha256_hex
    {
        return Err(ReqwestKparDownloadedError::DigestMismatch {
            url: response_url,
            expected: expected.sha256_hex.clone(),
            computed: computed_hash,
        });
    }
    let inner = LocalKParProjectRaw::new_tempdir(tmp_dir, archive_path, KparInnerPath::Guess)?;
    Ok((
        inner,
        KparMeta {
            size_bytes: written,
            sha256_hex: computed_hash,
        },
    ))
}

#[cfg(test)]
#[path = "./reqwest_kpar_download_tests.rs"]
mod tests;
