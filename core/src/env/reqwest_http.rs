// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{
    io,
    marker::{Send, Unpin},
    pin::Pin,
    string::String,
    sync::Arc,
};

use futures::{Stream, TryStreamExt};
use sha2::Sha256;
use thiserror::Error;

use crate::{
    auth::{HTTPAuthentication, StandardHTTPAuthentication},
    env::{
        AsSyncEnvironmentTokio, ReadEnvironmentAsync,
        local_directory::{ENTRIES_PATH, VERSIONS_PATH},
        segment_uri_generic,
    },
    project::{
        reqwest_kpar_download::ReqwestKparDownloadedProject, reqwest_src::ReqwestSrcProjectAsync,
    },
    resolve::{
        net_utils::{json_head_request, kpar_head_request, text_get_request},
        reqwest_http::HTTPProjectAsync,
    },
};

use futures::{AsyncBufReadExt as _, StreamExt as _};

pub type HTTPEnvironment = AsSyncEnvironmentTokio<HTTPEnvironmentAsync<StandardHTTPAuthentication>>;

#[derive(Debug)]
pub struct HTTPEnvironmentAsync<Policy> {
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    pub base_url: reqwest::Url,
    pub prefer_src: bool,
    // Currently no async implementation of ranged
    // pub try_ranged: bool,
}

#[derive(Error, Debug)]
pub enum HTTPEnvironmentError {
    #[error("failed to extend URL `{0}` with path `{1}`: {2}")]
    JoinURL(Box<str>, String, url::ParseError),
    // TODO: nicer formatting. Debug formatting is used here to include
    // all the details, since they are not given in the Display impl
    #[error("error making an HTTP request:\n{0:#?}")]
    HTTPRequest(#[from] reqwest_middleware::Error),
    #[error("failed to get project `{0}`, version `{1}` in source or kpar format")]
    InvalidURL(Box<str>, Box<str>),
    #[error("failed to read HTTP response: {0}")]
    HttpIo(io::Error),
}

pub fn path_encode_uri<S: AsRef<str>>(uri: S) -> std::vec::IntoIter<String> {
    segment_uri_generic::<S, Sha256>(uri)
}

impl<Policy: HTTPAuthentication> HTTPEnvironmentAsync<Policy> {
    pub fn root_url(&self) -> url::Url {
        let mut result = self.base_url.clone();

        if self.base_url.path() != "" {
            result
        } else {
            result.set_path("/");

            result
        }
    }

    pub fn url_join(url: &url::Url, join: &str) -> Result<url::Url, HTTPEnvironmentError> {
        url.join(join)
            .map_err(|e| HTTPEnvironmentError::JoinURL(url.as_str().into(), join.into(), e))
    }

    pub fn entries_url(&self) -> Result<url::Url, HTTPEnvironmentError> {
        Self::url_join(&self.root_url(), ENTRIES_PATH)
    }

    pub fn iri_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
        let mut result = self.root_url();

        for mut component in path_encode_uri(iri) {
            component.push('/');
            result = Self::url_join(&result, &component)?;
        }

        Ok(result)
    }

    pub fn iri_url_join<S: AsRef<str>>(
        &self,
        iri: S,
        join: &str,
    ) -> Result<url::Url, HTTPEnvironmentError> {
        let result = self.iri_url(iri)?;
        Self::url_join(&result, join)
    }

    pub fn versions_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
        self.iri_url_join(iri, VERSIONS_PATH)
    }

    pub fn project_kpar_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPEnvironmentError> {
        let join = format!("{}.kpar", version.as_ref());
        self.iri_url_join(iri, &join)
    }

    pub fn project_src_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPEnvironmentError> {
        let join = format!("{}.kpar/", version.as_ref());
        self.iri_url_join(iri, &join)
    }

    async fn try_get_project_src<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProjectAsync<Policy>>, HTTPEnvironmentError> {
        let project_url = self.project_src_url(uri, version)?;
        let src_project_url = Self::url_join(&project_url, ".project.json")?;

        let src_project_response = self
            .auth_policy
            .with_authentication(&self.client, &json_head_request(src_project_url.clone()))
            .await?;

        if !src_project_response.status().is_success() {
            return Ok(None);
        }

        Ok(Some(HTTPProjectAsync::HTTPSrcProject(
            ReqwestSrcProjectAsync {
                client: self.client.clone(),
                url: src_project_url,
                auth_policy: self.auth_policy.clone(),
            },
        )))
    }

    async fn try_get_project_kpar<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProjectAsync<Policy>>, HTTPEnvironmentError> {
        let kpar_project_url = self.project_kpar_url(&uri, &version)?;

        let kpar_project_response = self
            .auth_policy
            .with_authentication(&self.client, &kpar_head_request(kpar_project_url.clone()))
            .await?;

        if !kpar_project_response.status().is_success() {
            return Ok(None);
        }

        Ok(Some(HTTPProjectAsync::HTTPKParProjectDownloaded(
            ReqwestKparDownloadedProject::new_guess_root(
                &kpar_project_url,
                self.client.clone(),
                self.auth_policy.clone(),
            )
            .expect("internal IO error"),
        )))
    }
}

#[derive(Debug)]
pub struct Optionally<I> {
    inner: Option<I>,
}

impl<I: Iterator> Iterator for Optionally<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(inner) = &mut self.inner {
            inner.next()
        } else {
            None
        }
    }
}

impl<I: Stream + Unpin> Stream for Optionally<I> {
    type Item = I::Item;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

        if let Some(thing) = self.get_mut().inner.as_mut() {
            thing.poll_next_unpin(cx)
        } else {
            std::task::Poll::Ready(None)
        }
    }
}

fn trim_line<E>(line: Result<String, E>) -> Result<String, E> {
    Ok(line?.trim().to_string())
}

impl<Policy: HTTPAuthentication> ReadEnvironmentAsync for HTTPEnvironmentAsync<Policy> {
    type ReadError = HTTPEnvironmentError;

    // This can be made more concrete, but the type is humongous
    type UriStream = Optionally<
        Pin<Box<dyn futures::Stream<Item = Result<String, HTTPEnvironmentError>> + Send>>,
    >;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let response = self
            .auth_policy
            .with_authentication(&self.client, &text_get_request(self.entries_url()?))
            .await?;

        let inner = if response.status().is_success() {
            Some(
                response
                    .bytes_stream()
                    .map_err(io::Error::other)
                    .into_async_read()
                    .lines()
                    .map(trim_line)
                    .map_err(HTTPEnvironmentError::HttpIo)
                    .boxed(),
            )
        } else {
            None
        };

        Ok(Optionally { inner })
    }

    type VersionStream = Optionally<
        Pin<Box<dyn futures::Stream<Item = Result<String, HTTPEnvironmentError>> + Send>>,
    >;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        let response = self
            .auth_policy
            .with_authentication(
                &self.client,
                &text_get_request(self.versions_url(uri.as_ref())?),
            )
            .await?;

        let inner = if response.status().is_success() {
            Some(
                response
                    .bytes_stream()
                    .map_err(io::Error::other)
                    .into_async_read()
                    .lines()
                    .map(trim_line)
                    .map_err(HTTPEnvironmentError::HttpIo)
                    .boxed(),
            )
        } else {
            None
        };

        Ok(Optionally { inner })
    }

    type InterchangeProjectRead = HTTPProjectAsync<Policy>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        if self.prefer_src {
            if let Some(proj) = self.try_get_project_src(&uri, &version).await? {
                Ok(proj)
            } else if let Some(proj) = self.try_get_project_kpar(&uri, &version).await? {
                Ok(proj)
            } else {
                Err(HTTPEnvironmentError::InvalidURL(
                    uri.as_ref().into(),
                    version.as_ref().into(),
                ))
            }
        } else if let Some(proj) = self.try_get_project_kpar(&uri, &version).await? {
            Ok(proj)
        } else if let Some(proj) = self.try_get_project_src(&uri, &version).await? {
            Ok(proj)
        } else {
            Err(HTTPEnvironmentError::InvalidURL(
                uri.as_ref().into(),
                version.as_ref().into(),
            ))
        }
    }
}

#[cfg(test)]
#[path = "./reqwest_http_tests.rs"]
mod tests;
