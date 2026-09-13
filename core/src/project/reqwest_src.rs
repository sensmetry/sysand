// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

//! This module implements accessing interchanged projects stored remotely over HTTP.

use std::{io, marker::Send, pin::Pin, sync::Arc};

use fluent_uri::Iri;
use futures::{TryStreamExt as _, join};
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    manifest::diagnostics::Report,
    model::{
        InterchangeProjectInfoPartial, InterchangeProjectInfoRaw,
        InterchangeProjectMetadataPartial, InterchangeProjectMetadataRaw, PartialManifest,
    },
    project::{CanonicalizationError, ProjectReadAsync, utils::FsIoError},
    resolve::net_utils::{json_get_request, json_head_request, text_get_request},
};

use super::ProjectChecksum;

/// Project stored at a remote base-URL such as https://www.example.com/project/
/// (note that the final slash is semantically important!)
/// with info/meta accessible by HTTP GET-requests sent to
/// https://www.example.com/project/.project.json and
/// https://www.example.com/project/.meta.json.
/// Source file paths are expected to be accessible by GET request relative to the
/// base-path after (minimal) URL-encoding each path component. For example:
/// /Mëkanïk/Kömmandöh.sysml
/// is accessed by
/// GET https://www.example.com/project/M%C3%ABkan%C3%AFk/K%C3%B6mmand%C3%B6h.sysml
#[derive(Clone, Debug)]
pub struct ReqwestSrcProjectAsync<Policy> {
    /// (reqwest) HTTP client to use for GET requests
    pub client: reqwest_middleware::ClientWithMiddleware, // Internally an Arc
    /// Base-url of the project
    pub url: reqwest::Url,
    pub auth_policy: Arc<Policy>,
    pub expected_checksum: Option<String>,
}

impl<Policy> ReqwestSrcProjectAsync<Policy> {
    pub fn info_url(&self) -> reqwest::Url {
        self.url.join(".project.json").expect("internal URL error")
    }

    pub fn meta_url(&self) -> reqwest::Url {
        self.url.join(".meta.json").expect("internal URL error")
    }

    pub fn src_url<P: AsRef<Utf8UnixPath>>(&self, path: P) -> reqwest::Url {
        self.url
            .join(path.as_ref().as_str())
            .expect("internal URL error")
    }
}

#[derive(Error, Debug)]
pub enum ReqwestSrcError {
    #[error("error making an HTTP request:\n{0:#?}")]
    ReqwestMiddleware(reqwest_middleware::Error),
    #[error("error making an HTTP request:\n{0:#?}")]
    Reqwest(reqwest::Error),
    #[error("HTTP request to\n  `{0}`\n  returned malformed data: {1}")]
    Deserialize(String, serde_json::Error),
    #[error("HTTP request to\n  `{0}`\n  returned a document that is not a valid manifest:\n{1}")]
    InvalidManifest(String, Report),
    #[error("HTTP request to `{0}` returned unexpected status code {1}")]
    BadStatus(Box<str>, reqwest::StatusCode),
    #[error("project is missing `.project.json` and/or `.meta.json` files")]
    MissingInfoMeta,
    // This is only needed because `checksum_canonical_hex` can return
    // `io:Error`, which in this case is actually a network/HTTP error
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

impl<Policy: HTTPAuthentication> ReqwestSrcProjectAsync<Policy> {
    /// The body of one manifest document, or `None` when the server does
    /// not have it.
    async fn fetch(&self, url: reqwest::Url) -> Result<Option<String>, ReqwestSrcError> {
        let response = self
            .auth_policy
            .with_authentication(&self.client, &json_get_request(url))
            .await
            .map_err(ReqwestSrcError::ReqwestMiddleware)?;

        if !response.status().is_success() {
            return Ok(None);
        }
        response
            .text()
            .await
            .map(Some)
            .map_err(ReqwestSrcError::Reqwest)
    }

    /// One fetched document, parsed and validated; its warnings are
    /// logged against the URL they came from.
    fn validate<T: PartialManifest>(
        url: reqwest::Url,
        body: String,
        metamodel: Option<&str>,
    ) -> Result<T::Validated, ReqwestSrcError> {
        let partial: T = serde_json::from_str(&body)
            .map_err(|e| ReqwestSrcError::Deserialize(body.clone(), e))?;
        match partial.validate_manifest(metamodel) {
            Ok(validated) => {
                for warning in &validated.warnings {
                    log::warn!("`{url}`: {warning}");
                }
                Ok(validated.value)
            }
            Err(report) => Err(ReqwestSrcError::InvalidManifest(url.to_string(), report)),
        }
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for ReqwestSrcProjectAsync<Policy> {
    type Error = ReqwestSrcError;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        // Both documents are fetched at once, as before, and validated
        // afterwards: the metadata carries the metamodel a message about
        // the other document may need to name.
        let (info, meta) = join!(self.fetch(self.info_url()), self.fetch(self.meta_url()));
        let (info, meta) = (info?, meta?);

        let meta = meta
            .map(|body| {
                Self::validate::<InterchangeProjectMetadataPartial>(self.meta_url(), body, None)
            })
            .transpose()?;
        let metamodel = meta
            .as_ref()
            .and_then(|meta: &InterchangeProjectMetadataRaw| meta.metamodel.as_deref());
        let info = info
            .map(|body| {
                Self::validate::<InterchangeProjectInfoPartial>(self.info_url(), body, metamodel)
            })
            .transpose()?;

        Ok((info, meta))
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        // On its own, the information document has no sibling to take a
        // metamodel from, and fetching one for a message is not worth a
        // second request.
        self.fetch(self.info_url())
            .await?
            .map(|body| {
                Self::validate::<InterchangeProjectInfoPartial>(self.info_url(), body, None)
            })
            .transpose()
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        self.fetch(self.meta_url())
            .await?
            .map(|body| {
                Self::validate::<InterchangeProjectMetadataPartial>(self.meta_url(), body, None)
            })
            .transpose()
    }

    type SourceReader<'a>
        = futures::stream::IntoAsyncRead<
        Pin<Box<dyn futures::Stream<Item = Result<bytes::Bytes, io::Error>> + Send>>,
    >
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        use futures::StreamExt as _;

        let resp = self
            .auth_policy
            .with_authentication(&self.client, &text_get_request(self.src_url(path)))
            .await
            .map_err(ReqwestSrcError::ReqwestMiddleware)?;

        if resp.status().is_success() {
            Ok(resp
                .bytes_stream()
                .map_err(io::Error::other)
                .boxed()
                .into_async_read())
        } else {
            Err(ReqwestSrcError::BadStatus(
                resp.url().as_str().into(),
                resp.status(),
            ))
        }
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        let info_request = &json_head_request(self.info_url());
        let info_resp = self
            .auth_policy
            .with_authentication(&self.client, info_request);

        let meta_request = &json_head_request(self.meta_url());
        let meta_resp = self
            .auth_policy
            .with_authentication(&self.client, meta_request);

        match join!(info_resp, meta_resp) {
            (Ok(info_head), Ok(meta_head)) => {
                !info_head.status().is_success() || !meta_head.status().is_success()
            }
            _ => true,
        }
    }

    async fn sources_async(&self, _ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        let checksum = match &self.expected_checksum {
            Some(c) => c.clone(),
            None => self
                .checksum_canonical_hex_async()
                .await
                .map_err(|e| match e {
                    CanonicalizationError::ProjectRead(e) => e,
                    CanonicalizationError::FileRead(path, error) => ReqwestSrcError::Io(
                        FsIoError::ReadFile(String::from(path).into(), error).into(),
                    ),
                })?
                .ok_or(ReqwestSrcError::MissingInfoMeta)?,
        };
        Ok(vec![Source::RemoteSrc {
            // Panic here should be impossible, as the URL should have
            // been created from IRI
            remote_src: Iri::parse(self.url.to_string()).unwrap(),
            checksum,
        }])
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // A URL names one project.
        false
    }

    async fn checksum_canonical_variant_async(&self) -> Result<ProjectChecksum, Self::Error> {
        let checksum = self
            .checksum_canonical_hex_async()
            .await
            .map_err(|e| match e {
                CanonicalizationError::ProjectRead(e) => e,
                CanonicalizationError::FileRead(path, error) => ReqwestSrcError::Io(
                    FsIoError::ReadFile(String::from(path).into(), error).into(),
                ),
            })?
            .ok_or(ReqwestSrcError::MissingInfoMeta)?;
        Ok(ProjectChecksum::Project(checksum))
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
}

#[cfg(test)]
#[path = "./reqwest_src_tests.rs"]
mod tests;
