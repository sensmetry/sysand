// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{io, pin::Pin, sync::Arc};

use futures::AsyncRead;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    model::InterchangeProjectUsage,
    project::{
        CanonicalizationError, ProjectDefinition, ProjectReadAsync,
        reqwest_kpar_download::ReqwestRemoteKparDownloadedProject,
        reqwest_src::ReqwestSrcProjectAsync,
    },
    resolve::{ResolutionInfo, ResolveReadAsync},
    utils::scheme::{SCHEME_HTTP, SCHEME_HTTPS},
};

use super::ResolutionOutcome;

/// Tries to resolve http(s) URLs as direct (resolvable) links to interchange projects.
#[derive(Debug)]
pub struct HTTPResolverAsync<Policy> {
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub lax: bool,
    pub auth_policy: Arc<Policy>,
}

#[derive(Debug)]
pub enum HTTPProjectAsync<Policy> {
    HTTPSrcProject(ReqwestSrcProjectAsync<Policy>),
    HTTPKParProjectDownloaded(Box<ReqwestRemoteKparDownloadedProject<Policy>>),
}

#[derive(Error, Debug)]
pub enum HTTPProjectError<Policy: HTTPAuthentication> {
    #[error(transparent)]
    SrcProject(<ReqwestSrcProjectAsync<Policy> as ProjectReadAsync>::Error),
    #[error(transparent)]
    KparDownloaded(<ReqwestRemoteKparDownloadedProject<Policy> as ProjectReadAsync>::Error),
}

pub enum HTTPProjectAsyncReader<'a, Policy: HTTPAuthentication> {
    SrcProjectReader(<ReqwestSrcProjectAsync<Policy> as ProjectReadAsync>::SourceReader<'a>),
    KparDownloadedReader(
        <ReqwestRemoteKparDownloadedProject<Policy> as ProjectReadAsync>::SourceReader<'a>,
    ),
}

impl<Policy: HTTPAuthentication> AsyncRead for HTTPProjectAsyncReader<'_, Policy> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            HTTPProjectAsyncReader::SrcProjectReader(proj) => Pin::new(proj).poll_read(cx, buf),
            HTTPProjectAsyncReader::KparDownloadedReader(proj) => Pin::new(proj).poll_read(cx, buf),
        }
    }
}

impl<Policy: HTTPAuthentication> ProjectReadAsync for HTTPProjectAsync<Policy> {
    type Error = HTTPProjectError<Policy>;

    async fn get_project_async(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .get_project_async()
                .await
                .map_err(HTTPProjectError::SrcProject),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .get_project_async()
                .await
                .map_err(HTTPProjectError::KparDownloaded),
        }
    }

    type SourceReader<'a>
        = HTTPProjectAsyncReader<'a, Policy>
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => match proj.read_source_async(path).await {
                Ok(v) => Ok(HTTPProjectAsyncReader::SrcProjectReader(v)),
                Err(e) => Err(HTTPProjectError::SrcProject(e)),
            },
            Self::HTTPKParProjectDownloaded(proj) => match proj.read_source_async(path).await {
                Ok(v) => Ok(HTTPProjectAsyncReader::KparDownloadedReader(v)),
                Err(e) => Err(HTTPProjectError::KparDownloaded(e)),
            },
        }
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        match self {
            Self::HTTPSrcProject(proj) => proj.is_definitely_invalid_async().await,
            Self::HTTPKParProjectDownloaded(proj) => proj.is_definitely_invalid_async().await,
        }
    }

    async fn sources_async(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .sources_async(ctx)
                .await
                .map_err(HTTPProjectError::SrcProject),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .sources_async(ctx)
                .await
                .map_err(HTTPProjectError::KparDownloaded),
        }
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        match self {
            Self::HTTPSrcProject(proj) => proj.source_may_offer_multiple_versions(),
            Self::HTTPKParProjectDownloaded(proj) => proj.source_may_offer_multiple_versions(),
        }
    }

    async fn version_async(&self) -> Result<Option<String>, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .version_async()
                .await
                .map_err(HTTPProjectError::SrcProject),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .version_async()
                .await
                .map_err(HTTPProjectError::KparDownloaded),
        }
    }

    async fn usage_async(
        &self,
    ) -> Result<Option<Vec<crate::model::InterchangeProjectUsageRaw>>, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .usage_async()
                .await
                .map_err(HTTPProjectError::SrcProject),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .usage_async()
                .await
                .map_err(HTTPProjectError::KparDownloaded),
        }
    }

    async fn checksum_canonical_hex_async(
        &self,
    ) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .checksum_canonical_hex_async()
                .await
                .map_err(|e| e.map_project_read(HTTPProjectError::SrcProject)),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .checksum_canonical_hex_async()
                .await
                .map_err(|e| e.map_project_read(HTTPProjectError::KparDownloaded)),
        }
    }

    async fn checksum_canonical_variant_async(
        &self,
    ) -> Result<crate::project::ProjectChecksum, Self::Error> {
        match self {
            Self::HTTPSrcProject(proj) => proj
                .checksum_canonical_variant_async()
                .await
                .map_err(HTTPProjectError::SrcProject),
            Self::HTTPKParProjectDownloaded(proj) => proj
                .checksum_canonical_variant_async()
                .await
                .map_err(HTTPProjectError::KparDownloaded),
        }
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        match self {
            Self::HTTPSrcProject(proj) => proj.project_root(),
            Self::HTTPKParProjectDownloaded(proj) => proj.project_root(),
        }
    }
}

pub struct HTTPProjects<Policy> {
    client: reqwest_middleware::ClientWithMiddleware,
    url: reqwest::Url,
    src_done: bool,
    kpar_done: bool,
    // See the comments in `try_resolve_as_src`.
    lax: bool,
    auth_policy: Arc<Policy>,
}

impl<Policy: HTTPAuthentication> HTTPProjects<Policy> {
    pub fn try_resolve_as_kpar(&self) -> Option<HTTPProjectAsync<Policy>> {
        // TODO: Decide a policy for KPar vs Src urls
        let url = if self.url.path() == "" || !self.url.path().ends_with('/') {
            self.url.clone()
        // If the resolver is set to be lax, try dropping the terminal slash
        } else if self.lax {
            let mut lax_url = self.url.clone();
            let lax_path = lax_url.path().to_owned();
            lax_url.set_path(
                lax_path
                    .strip_suffix('/')
                    .expect("internal url processing error"),
            );

            lax_url
        } else {
            return None;
        };

        Some(HTTPProjectAsync::HTTPKParProjectDownloaded(Box::new(
            ReqwestRemoteKparDownloadedProject::new_guess_root(
                &url,
                self.client.clone(),
                self.auth_policy.clone(),
                None,
            )
            .expect("internal IO error"),
        )))
    }

    pub fn try_resolve_as_src(&self, auth_policy: Arc<Policy>) -> Option<HTTPProjectAsync<Policy>> {
        // These URLs should technically have a path that ends (explicitly or implicitly)
        // with a slash, due to the way relative references are treated in HTTP. E.g.:
        // resolving `bar` relative to `http://www.example.com/foo` gives `http://www.example.com/bar`
        // while resolving `bar` relative to `http://www.example.com/foo/` gives `http://www.example.com/foo/bar`
        if self.url.path() == "" || self.url.path().ends_with('/') {
            Some(HTTPProjectAsync::HTTPSrcProject(ReqwestSrcProjectAsync {
                client: self.client.clone(),
                url: self.url.clone(),
                auth_policy,
                expected_checksum: None,
            }))
        // If the resolver is set to be lax, try forcing the terminal slash
        } else if self.lax {
            let mut lax_url = self.url.clone();
            let mut lax_path = lax_url.path().to_owned();
            lax_path.push('/');
            lax_url.set_path(&lax_path);

            Some(HTTPProjectAsync::HTTPSrcProject(ReqwestSrcProjectAsync {
                client: self.client.clone(),
                url: lax_url,
                auth_policy,
                expected_checksum: None,
            }))
        } else {
            None
        }
    }
}

impl<Policy: HTTPAuthentication> Iterator for HTTPProjects<Policy> {
    type Item = Result<HTTPProjectAsync<Policy>, !>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.src_done {
            self.src_done = true;

            if let Some(proj) = self.try_resolve_as_src(self.auth_policy.clone()) {
                return Some(Ok(proj));
            }
        }

        if !self.kpar_done {
            self.kpar_done = true;

            if let Some(proj) = self.try_resolve_as_kpar() {
                return Some(Ok(proj));
            }
        }

        None
    }
}

/// Tries treat IRIs as HTTP URLs, pointing either to source files stored remotely
/// or a KPAR archive stored remotely.
impl<Policy: HTTPAuthentication> ResolveReadAsync for HTTPResolverAsync<Policy> {
    type Error = !;

    type ProjectStorage = HTTPProjectAsync<Policy>;

    type ResolvedStorages = futures::stream::Iter<HTTPProjects<Policy>>;

    async fn resolve_read_async(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        // Try to resolve as a HTTP src project.
        let outcome = match resolve.usage() {
            InterchangeProjectUsage::Resource {
                resource: iri,
                version_constraint: _,
            } => {
                if iri.scheme() == SCHEME_HTTP || iri.scheme() == SCHEME_HTTPS {
                    match reqwest::Url::parse(iri.as_str()) {
                        Ok(url) => {
                            ResolutionOutcome::Resolved(futures::stream::iter(HTTPProjects {
                                client: self.client.clone(),
                                url,
                                src_done: false,
                                kpar_done: false,
                                lax: self.lax,
                                auth_policy: self.auth_policy.clone(),
                            }))
                        }
                        Err(e) => ResolutionOutcome::Unresolvable {
                            reason: format!("invalid http(s) URL: {e}"),
                        },
                    }
                } else {
                    ResolutionOutcome::UnsupportedUsageType {
                        reason: String::from("not an http(s) URL"),
                    }
                }
            }
            _ => ResolutionOutcome::UnsupportedUsageType {
                reason: String::from("not a url/resource usage"),
            },
        };
        Ok(outcome)
    }
}

#[cfg(test)]
#[path = "./reqwest_http_tests.rs"]
mod tests;
