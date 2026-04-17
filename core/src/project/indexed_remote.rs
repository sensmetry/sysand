// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use thiserror::Error;
use tokio::sync::OnceCell;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsageRaw,
        project_hash_raw,
    },
    project::{
        CanonicalizationError, ProjectReadAsync,
        reqwest_kpar_download::{ReqwestKparDownloadedError, ReqwestKparDownloadedProject},
    },
    resolve::net_utils::json_get_request,
};

type LockedProjectJson = (InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw);

#[derive(Debug)]
pub struct IndexedRemoteProject<Policy> {
    pub(crate) downloaded: ReqwestKparDownloadedProject<Policy>,
    pub(crate) inline_version: String,
    pub(crate) inline_usage: Vec<InterchangeProjectUsageRaw>,
    pub(crate) project_json_url: reqwest::Url,
    pub(crate) meta_json_url: reqwest::Url,
    pub(crate) fetched_project: OnceCell<LockedProjectJson>,
    pub(crate) kpar_size: u64,
    pub(crate) expected_project_digest: String,
    pub(crate) expected_kpar_digest: String,
}

#[derive(Error, Debug)]
pub enum IndexedRemoteProjectError {
    #[error("HTTP request to `{url}` returned status {status}")]
    BadHttpStatus {
        url: Box<str>,
        status: reqwest::StatusCode,
    },
    #[error("failed to read HTTP response from `{url}`: {source}")]
    ResponseBody {
        url: Box<str>,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to parse JSON from `{url}`: {source}")]
    JsonParse {
        url: Box<str>,
        #[source]
        source: serde_json::Error,
    },
    #[error("project selection metadata at `{url}` disagrees with versions.json for `{field}`")]
    SelectionDrift { url: Box<str>, field: &'static str },
    #[error(
        "project at `{url}` has locally-computed canonical digest `{computed}` \
         but the expected digest was `{expected}`"
    )]
    ProjectDigestDrift {
        url: Box<str>,
        expected: String,
        computed: String,
    },
    #[error(transparent)]
    Downloaded(#[from] ReqwestKparDownloadedError),
    #[error("error making an HTTP request:\n{0:#?}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
}

impl<Policy: HTTPAuthentication> IndexedRemoteProject<Policy> {
    pub fn new(
        url: reqwest::Url,
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        version: String,
        usage: Vec<InterchangeProjectUsageRaw>,
        project_json_url: reqwest::Url,
        meta_json_url: reqwest::Url,
        kpar_size: u64,
        expected_project_digest: String,
        expected_kpar_digest: String,
    ) -> Result<Self, IndexedRemoteProjectError> {
        Ok(Self {
            downloaded: ReqwestKparDownloadedProject::new(url, client, auth_policy)?,
            inline_version: version,
            inline_usage: usage,
            project_json_url,
            meta_json_url,
            fetched_project: OnceCell::new(),
            kpar_size,
            expected_project_digest,
            expected_kpar_digest,
        })
    }

    async fn fetch_required_json<T: serde::de::DeserializeOwned>(
        &self,
        url: reqwest::Url,
    ) -> Result<T, IndexedRemoteProjectError> {
        let response = self
            .downloaded
            .auth_policy
            .with_authentication(&self.downloaded.client, &json_get_request(url.clone()))
            .await?;

        if !response.status().is_success() {
            return Err(IndexedRemoteProjectError::BadHttpStatus {
                url: url.as_str().into(),
                status: response.status(),
            });
        }

        let bytes =
            response
                .bytes()
                .await
                .map_err(|source| IndexedRemoteProjectError::ResponseBody {
                    url: url.as_str().into(),
                    source,
                })?;

        serde_json::from_slice(&bytes).map_err(|source| IndexedRemoteProjectError::JsonParse {
            url: url.as_str().into(),
            source,
        })
    }

    async fn fetched_project_async(&self) -> Result<&LockedProjectJson, IndexedRemoteProjectError> {
        self.fetched_project
            .get_or_try_init(|| async {
                let (info, meta): (
                    InterchangeProjectInfoRaw,
                    InterchangeProjectMetadataRaw,
                ) = futures::try_join!(
                    self.fetch_required_json(self.project_json_url.clone()),
                    self.fetch_required_json(self.meta_json_url.clone()),
                )?;

                if info.version != self.inline_version {
                    return Err(IndexedRemoteProjectError::SelectionDrift {
                        url: self.project_json_url.as_str().into(),
                        field: "version",
                    });
                }

                if info.usage != self.inline_usage {
                    return Err(IndexedRemoteProjectError::SelectionDrift {
                        url: self.project_json_url.as_str().into(),
                        field: "usage",
                    });
                }

                let computed = format!("{:x}", project_hash_raw(&info, &meta));
                if computed != self.expected_project_digest {
                    return Err(IndexedRemoteProjectError::ProjectDigestDrift {
                        url: self.project_json_url.as_str().into(),
                        expected: self.expected_project_digest.clone(),
                        computed,
                    });
                }

                Ok((info, meta))
            })
            .await
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for IndexedRemoteProject<Policy> {
    type Error = IndexedRemoteProjectError;

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
        self.downloaded
            .ensure_downloaded_with_sha256_digest(Some(&self.expected_kpar_digest))
            .await?;

        self.downloaded
            .read_source_async(path)
            .await
            .map_err(Into::into)
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(vec![Source::RemoteKpar {
            remote_kpar: self.downloaded.url.to_string(),
            remote_kpar_size: Some(self.kpar_size),
        }])
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        Ok(Some(self.fetched_project_async().await?.0.clone()))
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        Ok(Some(self.fetched_project_async().await?.1.clone()))
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        Ok(Some(self.inline_version.clone()))
    }

    async fn usage_async(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        Ok(Some(self.inline_usage.clone()))
    }

    async fn checksum_canonical_hex_async(
        &self,
    ) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        if !self.downloaded.inner.archive_path.is_file() {
            return Ok(Some(self.expected_project_digest.clone()));
        }

        let computed = self
            .downloaded
            .checksum_canonical_hex_async()
            .await
            .map_err(|e| e.map_project_read(IndexedRemoteProjectError::Downloaded))?;

        if let Some(computed_hex) = computed.as_ref()
            && computed_hex != &self.expected_project_digest
        {
            return Err(CanonicalizationError::ProjectRead(
                IndexedRemoteProjectError::ProjectDigestDrift {
                    url: self.project_json_url.as_str().into(),
                    expected: self.expected_project_digest.clone(),
                    computed: computed_hex.clone(),
                },
            ));
        }

        Ok(computed)
    }
}
