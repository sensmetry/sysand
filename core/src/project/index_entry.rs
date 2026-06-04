// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP-backed project seeded with metadata advertised in `versions.json`.
//!
//! `IndexEntryProject` is the `ProjectRead`/`ProjectReadAsync` leaf returned
//! by [`crate::env::index::IndexEnvironmentAsync`] once a concrete version
//! has been selected. The protocol rules are documented in
//! `design/index-protocol.md`; this module only calls out the
//! implementation split:
//!
//! - Advertised-tier reads ([`ProjectReadAsync::version_async`],
//!   [`ProjectReadAsync::usage_async`]) returning fields from
//!   [`crate::env::index::AdvertisedVersion`] without I/O. Before the archive
//!   is verified, [`ProjectReadAsync::checksum_canonical_hex_async`] also
//!   returns the advertised digest directly.
//! - Lazily-fetched reads
//!   ([`ProjectReadAsync::get_project_async`]/`get_info_async`/`get_meta_async`)
//!   guarded by `fetched_info_meta`'s `OnceCell`.
//! - [`ProjectReadAsync::read_source_async`] delegating to
//!   [`crate::project::reqwest_kpar_download::ReqwestKparDownloadedProject`],
//!   which verifies the archive against the advertised `kpar_digest` before
//!   exposing source bytes.

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
        ProjectReadAsync,
        reqwest_kpar_download::{
            ReqwestIndexKparDownloadedProject, ReqwestKparDownloadedError,
            ReqwestRemoteKparDownloadedProject,
        },
    },
};

use super::ProjectChecksum;

#[derive(Debug)]
pub struct IndexEntryProject<Policy> {
    /// The kpar archive backend — field name tracks its role in this struct,
    /// type name tracks the transport.
    pub(crate) archive: ReqwestIndexKparDownloadedProject<Policy>,
    /// Single source of truth for protocol-advertised per-version metadata.
    /// `version_async` and `usage_async` return these fields without I/O;
    /// `checksum_canonical_hex_async` does the same until the archive has been
    /// verified.
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
    #[error(transparent)]
    Downloaded(#[from] ReqwestKparDownloadedError),
}

impl<Policy: HTTPAuthentication> IndexEntryProject<Policy> {
    /// Construct a project wrapper for a version that has already been
    /// selected out of `versions.json`. URL arguments are ordered to
    /// match the protocol sequence: archive, project manifest, metadata.
    pub(crate) fn new(
        kpar_url: reqwest::Url,
        project_json_url: reqwest::Url,
        meta_json_url: reqwest::Url,
        advertised: AdvertisedVersion,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
    ) -> Result<Self, IndexEntryProjectError> {
        Ok(Self {
            archive: ReqwestIndexKparDownloadedProject::new(
                kpar_url,
                client,
                auth_policy,
                advertised.kpar_size,
                advertised.kpar_digest.as_hex().to_owned(),
            )?,
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

    /// Downloads `.project.json` and `.meta.json`. No verification is done.
    async fn ensure_downloaded(
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
        let (info, meta) = self.ensure_downloaded().await?;
        Ok((Some(info.clone()), Some(meta.clone())))
    }

    type SourceReader<'a>
        = <ReqwestRemoteKparDownloadedProject<Policy> as ProjectReadAsync>::SourceReader<'a>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.archive
            .read_source_async(path)
            .await
            .map_err(Into::into)
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::IndexKpar {
            index_kpar: self.archive.url().to_string(),
            kpar_size: self.advertised.kpar_size,
            kpar_digest: self.advertised.kpar_digest.as_hex().to_string(),
        }])
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(Some(self.ensure_downloaded().await?.0.clone()))
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(Some(self.ensure_downloaded().await?.1.clone()))
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        Ok(Some(self.advertised.version.to_string()))
    }

    async fn usage_async(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(Some(self.advertised.usage.clone()))
    }

    // TODO: decide the security requirements here and maybe have separate methods
    // used for e.g. generating a lockfile, where advertised checksum is fine,
    // as it will be verified on sync, and the actual verification of e.g. projects being downloaded
    // against a lockfile, or verifying that the correct ones are installed in env
    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        Ok(ProjectChecksum::Kpar(
            self.advertised.kpar_digest.as_hex().to_string(),
        ))
    }
}

#[cfg(test)]
#[path = "./index_entry_tests.rs"]
mod tests;
