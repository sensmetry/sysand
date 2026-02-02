// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    io,
    marker::{Send, Unpin},
    pin::Pin,
    string::String,
    sync::Arc,
};

use futures::{Stream, TryStreamExt};
use reqwest_middleware::ClientWithMiddleware;
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
    resolve::reqwest_http::HTTPProjectAsync,
};

use futures::{AsyncBufReadExt as _, StreamExt as _};

pub type HTTPEnvironment = AsSyncEnvironmentTokio<HTTPEnvironmentAsync<StandardHTTPAuthentication>>;

#[derive(Debug)]
pub struct HTTPEnvironmentAsync<Policy> {
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Pol>,
    pub base_url: reqwest::Url,
    pub prefer_src: bool,
    // Currently no async implementation of ranged
    // pub try_ranged: bool,
}

#[derive(Error, Debug)]
pub enum HTTPEnvironmentError {
    #[error("failed to extend URL `{0}` with path `{1}`: {2}")]
    JoinURL(Box<str>, String, url::ParseError),
    // TODO: include error.source(). Currently it gives no details what's gone
    // wrong. Also it includes URL, so no need to have it separately
    #[error("error making an HTTP request to '{0}':\n{1}")]
    HTTPRequest(Box<str>, reqwest_middleware::Error),
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

        let this_url = src_project_url.clone();
        let src_project_request = move |client: &ClientWithMiddleware| {
            client
                .head(this_url.clone())
                .header("ACCEPT", "application/json, text/plain")
        };
        let src_project_response = self
            .auth_policy
            .with_authentication(&self.client, &src_project_request)
            .await
            .map_err(|e| HTTPEnvironmentError::HTTPRequest(src_project_url.as_str().into(), e))?;

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

        let this_url = kpar_project_url.clone();
        let kpar_project_request = move |client: &ClientWithMiddleware| {
            client
                .head(this_url.clone())
                .header("ACCEPT", "application/zip, application/octet-stream")
        };
        let kpar_project_response = self
            .auth_policy
            .with_authentication(&self.client, &kpar_project_request)
            .await;

        if !kpar_project_response
            .map_err(|e| HTTPEnvironmentError::HTTPRequest(kpar_project_url.as_str().into(), e))?
            .status()
            .is_success()
        {
            return Ok(None);
        }

        Ok(Some(HTTPProjectAsync::HTTPKParProjectDownloaded(
            ReqwestKparDownloadedProject::new_guess_root(
                &self.project_kpar_url(&uri, &version)?,
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
        let this_url = self.entries_url()?;

        let response = self
            .auth_policy
            .with_authentication(&self.client, &move |client| client.get(this_url.clone()))
            .await
            .map_err(|e| {
                HTTPEnvironmentError::HTTPRequest(self.entries_url().unwrap().as_str().into(), e)
            })?;

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
        let this_url = self.versions_url(uri.as_ref())?;
        let response = self
            .auth_policy
            .with_authentication(&self.client, &move |client| client.get(this_url.clone()))
            .await
            .map_err(|e| {
                HTTPEnvironmentError::HTTPRequest(
                    self.versions_url(uri).unwrap().as_str().into(),
                    e,
                )
            })?;

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
mod test {
    use std::sync::Arc;

    use crate::{
        auth::Unauthenticated,
        env::{ReadEnvironment, ReadEnvironmentAsync},
        resolve::reqwest_http::HTTPProjectAsync,
    };

    #[test]
    fn test_uri_examples() -> Result<(), Box<dyn std::error::Error>> {
        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse("https://www.example.com/a/b")?,
            prefer_src: true,
            auth_policy: Arc::new(Unauthenticated {}),
            // try_ranged: false,
        };

        assert_eq!(env.root_url().to_string(), "https://www.example.com/a/b");
        assert_eq!(
            env.entries_url()?.to_string(),
            "https://www.example.com/a/entries.txt"
        );
        assert_eq!(
            env.versions_url("urn:kpar:b")?.to_string(),
            "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.txt"
        );
        assert_eq!(
            env.project_kpar_url("urn:kpar:b", "1.0.0")?.to_string(),
            "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0.kpar"
        );
        assert_eq!(
            env.project_src_url("urn:kpar:b", "1.0.0")?.to_string(),
            "https://www.example.com/a/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/1.0.0.kpar/"
        );

        Ok(())
    }

    #[test]
    fn test_basic_enumerations() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            auth_policy: Arc::new(Unauthenticated {}),
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        ));

        let entries_mock = server
            .mock("GET", "/entries.txt")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("urn:kpar:a\nurn:kpar:b")
            .create();

        let versions_a_mock = server
            .mock(
                "GET",
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/versions.txt",
            )
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("1.0.0")
            .create();

        let versions_b_mock = server
            .mock(
                "GET",
                "/621a5fdf587a3ecc878a98c8be2240dd5bbe561860d11f4da1ece4a4fe2fb8b5/versions.txt",
            )
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("1.0.0\n2.0.0")
            .create();

        let uris: Result<Vec<_>, _> = env.uris()?.collect();
        let uris = uris?;

        assert_eq!(uris.len(), 2);
        assert!(uris.contains(&"urn:kpar:a".to_string()));
        assert!(uris.contains(&"urn:kpar:b".to_string()));

        let a_versions: Result<Vec<_>, _> = env.versions("urn:kpar:a")?.collect();
        let a_versions = a_versions?;

        assert_eq!(a_versions.len(), 1);
        assert!(a_versions.contains(&"1.0.0".to_string()));

        let b_versions: Result<Vec<_>, _> = env.versions("urn:kpar:b")?.collect();
        let b_versions = b_versions?;

        assert_eq!(b_versions.len(), 2);
        assert!(b_versions.contains(&"1.0.0".to_string()));
        assert!(b_versions.contains(&"2.0.0".to_string()));

        entries_mock.assert();
        versions_a_mock.assert();
        versions_b_mock.assert();

        Ok(())
    }

    #[test]
    fn test_kpar_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            auth_policy: Arc::new(Unauthenticated {}),
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        ));

        let kpar_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
            )
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body("")
            .create();

        let project = env.get_project("urn:kpar:a", "1.0.0")?;

        let HTTPProjectAsync::HTTPKParProjectDownloaded(_) = project.inner else {
            panic!("Expected to resolve to KPar project");
        };

        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn test_src_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse(&host)?,
            prefer_src: false,
            auth_policy: Arc::new(Unauthenticated {}),
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        ));

        let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .create();

        let project = env.get_project("urn:kpar:a", "1.0.0")?;

        let HTTPProjectAsync::HTTPSrcProject(_) = project.inner else {
            panic!("Expected to resolve to src project");
        };

        src_mock.assert();

        Ok(())
    }

    #[test]
    fn test_kpar_preference() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse(&host)?,
            prefer_src: false,
            auth_policy: Arc::new(Unauthenticated {}),
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        ));

        let kpar_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
            )
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body("")
            .create();

        let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .expect_at_most(0)
            .create();

        let project = env.get_project("urn:kpar:a", "1.0.0")?;

        let HTTPProjectAsync::HTTPKParProjectDownloaded(_) = project.inner else {
            panic!("Expected to resolve to KPar project");
        };

        src_mock.assert();
        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn test_src_preference() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            auth_policy: Arc::new(Unauthenticated {}),
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        ));

        let kpar_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar",
            )
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body("")
            .expect_at_most(0)
            .create();

        let src_mock = server
            .mock(
                "HEAD", // urn:kpar:a
                "/cf642a3abc90961d460b5159fb9dd82c011a659bb59d1df976233675b23d78b0/1.0.0.kpar/.project.json",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("")
            .create();

        let project = env.get_project("urn:kpar:a", "1.0.0")?;

        let HTTPProjectAsync::HTTPSrcProject(_) = project.inner else {
            panic!("Expected to resolve to src project");
        };

        src_mock.assert();
        kpar_mock.assert();

        Ok(())
    }
}
