// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP-backed project seeded with metadata advertised in `versions.json`.
//!
//! `IndexEntryProject` is the `ProjectRead`/`ProjectReadAsync` leaf returned
//! by [`crate::env::index::IndexEnvironmentAsync`] once a concrete version
//! has been selected. The three-tier trust model (advertised /
//! lazily-fetched / lazily-fetched-and-verified) and the digest reconciliation
//! rules are documented in `docs/src/index-protocol.md`; this module is
//! the client-side implementation of them, with:
//!
//! - Advertised-tier reads ([`ProjectReadAsync::version_async`],
//!   [`ProjectReadAsync::usage_async`],
//!   [`ProjectReadAsync::checksum_canonical_hex_async`]) returning fields
//!   from [`crate::env::index::AdvertisedVersion`] without I/O.
//! - Lazily-fetched reads
//!   ([`ProjectReadAsync::get_project_async`]/`get_info_async`/`get_meta_async`)
//!   guarded by `fetched_info_meta`'s `OnceCell` so concurrent callers fan
//!   in to a single fetch + digest-verification pass. Digest
//!   verification against the advertised `project_digest` is mandatory
//!   before either document is exposed to callers, and the server is
//!   authoritative for textual fields — the client does not
//!   cross-check `version`/`usage` between `versions.json` and
//!   `.project.json`.
//! - [`ProjectReadAsync::read_source_async`] delegating to
//!   [`crate::project::reqwest_kpar_download::ReqwestKparDownloadedProject`],
//!   which verifies the streamed kpar body against the advertised
//!   `kpar_digest` before renaming into the verified path.
//! - [`IndexEntryProjectError::AdvertisedDigestDrift`] as the concrete
//!   error surface for digest mismatches, raised both pre-download (from
//!   the inline canonical digest — see
//!   [`crate::project::canonical_project_digest_inline`]) and post-download
//!   (authoritative check using the downloaded archive).

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::OnceCell;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    env::index::{AdvertisedVersion, HttpFetchError, MissingPolicy, fetch_json},
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw},
    project::{
        CanonicalizationError, ProjectReadAsync, canonical_project_digest_inline,
        reqwest_kpar_download::{ReqwestKparDownloadedError, ReqwestKparDownloadedProject},
    },
};

#[derive(Debug)]
pub struct IndexEntryProject<Policy> {
    /// The kpar archive backend — field name tracks its role in this struct,
    /// type name tracks the transport.
    pub(crate) archive: ReqwestKparDownloadedProject<Policy>,
    /// Single source of truth for the protocol-advertised per-version
    /// metadata. All `version_async`/`usage_async`/`checksum_canonical_hex_async`
    /// accesses return slices/clones of these fields without any I/O.
    pub(crate) advertised: AdvertisedVersion,
    pub(crate) project_json_url: reqwest::Url,
    pub(crate) meta_json_url: reqwest::Url,
    pub(crate) fetched_info_meta:
        OnceCell<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>,
}

#[derive(Error, Debug)]
pub enum IndexEntryProjectError {
    #[error(transparent)]
    Fetch(#[from] HttpFetchError),
    #[error(
        "project at `{url}` has locally-computed canonical digest `{computed}` \
         but the expected digest was `{expected}`"
    )]
    AdvertisedDigestDrift {
        url: Box<str>,
        expected: String,
        computed: String,
    },
    #[error(transparent)]
    Downloaded(#[from] ReqwestKparDownloadedError),
}

impl<Policy: HTTPAuthentication> IndexEntryProject<Policy> {
    /// Construct a project wrapper for a version that has already been
    /// selected out of `versions.json`. URL arguments are in
    /// archive → manifest → meta order (`kpar_url`, `project_json_url`,
    /// `meta_json_url`) so transposition across the three is hard to do
    /// silently.
    pub(crate) fn new(
        kpar_url: reqwest::Url,
        project_json_url: reqwest::Url,
        meta_json_url: reqwest::Url,
        advertised: AdvertisedVersion,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
    ) -> Result<Self, IndexEntryProjectError> {
        Ok(Self {
            archive: ReqwestKparDownloadedProject::new(kpar_url, client, auth_policy)?,
            advertised,
            project_json_url,
            meta_json_url,
            fetched_info_meta: OnceCell::new(),
        })
    }

    async fn fetch_required_json<T: serde::de::DeserializeOwned>(
        &self,
        url: reqwest::Url,
    ) -> Result<T, IndexEntryProjectError> {
        // RequirePresent — once a version is selected, a per-version
        // 404 is a hard error. The `.expect` is valid because
        // RequirePresent only returns `Ok(None)` under
        // `AllowNotFound`, which we're not using here.
        Ok(fetch_json(
            &self.archive.client,
            &*self.archive.auth_policy,
            &url,
            MissingPolicy::RequirePresent,
        )
        .await?
        .expect("RequirePresent never returns Ok(None)"))
    }

    async fn fetched_project_async(
        &self,
    ) -> Result<&(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), IndexEntryProjectError>
    {
        self.fetched_info_meta
            .get_or_try_init(|| async {
                let (info, meta): (
                    InterchangeProjectInfoRaw,
                    InterchangeProjectMetadataRaw,
                ) = futures::try_join!(
                    self.fetch_required_json(self.project_json_url.clone()),
                    self.fetch_required_json(self.meta_json_url.clone()),
                )?;

                // Pre-expose digest check (see module doc). A `None`
                // return from `canonical_project_digest_inline` means
                // `.meta.json` carries a non-SHA256 checksum entry
                // whose canonical form would require reading source
                // bytes from the kpar — a protocol violation, since
                // the canonical form is defined to be computable from
                // (info, meta) alone. Refuse to expose the document
                // rather than silently skipping verification.
                let Some(hash) = canonical_project_digest_inline(&info, &meta) else {
                    return Err(IndexEntryProjectError::AdvertisedDigestDrift {
                        url: self.project_json_url.as_str().into(),
                        expected: self.advertised.project_digest.as_hex().to_string(),
                        computed: "<uncomputable: meta.checksum uses an \
                                   unsupported (non-SHA256) algorithm>"
                            .to_string(),
                    });
                };
                let computed = format!("{:x}", hash);
                if computed != self.advertised.project_digest.as_hex() {
                    return Err(IndexEntryProjectError::AdvertisedDigestDrift {
                        url: self.project_json_url.as_str().into(),
                        expected: self.advertised.project_digest.as_hex().to_string(),
                        computed,
                    });
                }

                Ok((info, meta))
            })
            .await
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for IndexEntryProject<Policy> {
    type Error = IndexEntryProjectError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let (info, meta) = self.fetched_project_async().await?;
        Ok((Some(info.clone()), Some(meta.clone())))
    }

    type SourceReader<'a>
        = <ReqwestKparDownloadedProject<Policy> as ProjectReadAsync>::SourceReader<'a>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.archive
            .ensure_downloaded_verified(self.advertised.kpar_digest.as_hex())
            .await?;

        self.archive
            .read_source_async(path)
            .await
            .map_err(Into::into)
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::IndexKpar {
            index_kpar: self.archive.url.to_string(),
            index_kpar_size: self.advertised.kpar_size,
            index_kpar_digest: self.advertised.kpar_digest.as_hex().to_string(),
        }])
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(Some(self.fetched_project_async().await?.0.clone()))
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(Some(self.fetched_project_async().await?.1.clone()))
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        Ok(Some(self.advertised.version.to_string()))
    }

    async fn usage_async(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(Some(self.advertised.usage.clone()))
    }

    async fn checksum_canonical_hex_async(
        &self,
    ) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        // Gate on "kpar_digest has been verified", not merely "bytes
        // exist at the archive path". The post-download cross-check
        // below hashes (info, meta) read out of the kpar and compares
        // it against `advertised.project_digest`; running that over
        // bytes that were renamed into place by an *unverified*
        // `ensure_downloaded` path would mean the equality check only
        // detects a hash collision, not tampering. Today no caller in
        // this crate wires the archive through the unverified path,
        // but `ReqwestKparDownloadedProject::ProjectReadAsync` still
        // exposes one — gate here rather than rely on the invariant
        // holding forever.
        if !self.archive.is_verified() {
            return Ok(Some(self.advertised.project_digest.as_hex().to_string()));
        }

        let computed = self
            .archive
            .checksum_canonical_hex_async()
            .await
            .map_err(|e| e.map_project_read(IndexEntryProjectError::Downloaded))?;

        if let Some(computed_hex) = computed.as_ref()
            && computed_hex.as_str() != self.advertised.project_digest.as_hex()
        {
            return Err(CanonicalizationError::ProjectRead(
                IndexEntryProjectError::AdvertisedDigestDrift {
                    url: self.project_json_url.as_str().into(),
                    expected: self.advertised.project_digest.as_hex().to_string(),
                    computed: computed_hex.clone(),
                },
            ));
        }

        Ok(computed)
    }
}

#[cfg(test)]
#[path = "./index_entry_tests.rs"]
mod tests;
