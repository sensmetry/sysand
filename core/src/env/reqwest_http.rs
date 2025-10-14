// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::BufReader;

use futures::{Stream, TryStreamExt};
use sha2::Sha256;
use thiserror::Error;

use crate::{
    env::{
        AsSyncEnvironmentTokio, ReadEnvironmentAsync,
        local_directory::{ENTRIES_PATH, VERSIONS_PATH},
        segment_uri_generic,
    },
    project::{
        ProjectRead, reqwest_kpar_download::ReqwestKparDownloadedProject,
        reqwest_src::ReqwestSrcProjectAsync,
    },
    resolve::reqwest_http::HTTPProjectAsync,
};

use futures::{AsyncBufReadExt as _, StreamExt as _};

// #[derive(Debug)]
// pub struct HTTPEnvironment {
//     pub client: reqwest::blocking::Client,
//     pub base_url: reqwest::Url,
//     pub prefer_src: bool,
//     pub try_ranged: bool,
// }

pub type HTTPEnvironment = AsSyncEnvironmentTokio<HTTPEnvironmentAsync>;

#[derive(Debug)]
pub struct HTTPEnvironmentAsync {
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub base_url: reqwest::Url,
    pub prefer_src: bool,
    // Currently no async implementation of ranged
    // pub try_ranged: bool,
}

#[derive(Error, Debug)]
pub enum HTTPEnvironmentError {
    #[error("{0}")]
    URLError(#[from] url::ParseError),
    #[error("{0}")]
    ReqwestMiddlewareError(#[from] reqwest_middleware::Error),
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    IOError(#[from] std::io::Error),
    #[error("Unable to handle URL: {0}")]
    InvalidURL(url::Url),
}

pub fn path_encode_uri<S: AsRef<str>>(uri: S) -> std::vec::IntoIter<std::string::String> {
    segment_uri_generic::<S, Sha256>(uri)
}

// impl HTTPEnvironment {
//     pub fn root_url(&self) -> url::Url {
//         let mut result = self.base_url.clone();

//         if self.base_url.path() != "" {
//             result
//         } else {
//             result.set_path("/");

//             result
//         }
//     }

//     pub fn entries_url(&self) -> Result<url::Url, HTTPEnvironmentError> {
//         Ok(self.root_url().join(ENTRIES_PATH)?)
//     }

//     pub fn get_entries_request(
//         &self,
//     ) -> Result<reqwest::blocking::RequestBuilder, HTTPEnvironmentError> {
//         Ok(self.client.get(self.entries_url()?))
//     }

//     pub fn iri_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
//         let mut result = self.root_url();

//         for mut component in path_encode_uri(iri) {
//             component.push('/');
//             result = result.join(&component)?;
//         }

//         Ok(result)
//     }

//     pub fn versions_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
//         Ok(self.iri_url(iri)?.join(VERSIONS_PATH)?)
//     }

//     pub fn get_versions_request<S: AsRef<str>>(
//         &self,
//         iri: S,
//     ) -> Result<reqwest::blocking::RequestBuilder, HTTPEnvironmentError> {
//         Ok(self.client.get(self.iri_url(iri)?.join(VERSIONS_PATH)?))
//     }

//     pub fn project_kpar_url<S: AsRef<str>, T: AsRef<str>>(
//         &self,
//         iri: S,
//         version: T,
//     ) -> Result<url::Url, HTTPEnvironmentError> {
//         Ok(self
//             .iri_url(iri)?
//             .join(&format!("{}.kpar", version.as_ref()))?)
//     }

//     pub fn project_src_url<S: AsRef<str>, T: AsRef<str>>(
//         &self,
//         iri: S,
//         version: T,
//     ) -> Result<url::Url, HTTPEnvironmentError> {
//         Ok(self
//             .iri_url(iri)?
//             .join(&format!("{}.kpar/", version.as_ref()))?)
//     }

//     fn try_get_project_src<S: AsRef<str>, T: AsRef<str>>(
//         &self,
//         uri: S,
//         version: T,
//     ) -> Result<Option<HTTPProject>, HTTPEnvironmentError> {
//         let src_project_url = self.project_src_url(uri, version)?.join(".project.json")?;

//         if !self
//             .client
//             .head(src_project_url.clone())
//             .header("ACCEPT", "application/json, text/plain")
//             .send()?
//             .status()
//             .is_success()
//         {
//             return Ok(None);
//         }

//         Ok(Some(HTTPProject::HTTPSrcProject(ReqwestSrcProject {
//             client: self.client.clone(),
//             url: src_project_url,
//         })))
//     }

//     fn try_get_project_kpar<S: AsRef<str>, T: AsRef<str>>(
//         &self,
//         uri: S,
//         version: T,
//     ) -> Result<Option<HTTPProject>, HTTPEnvironmentError> {
//         let kpar_project_url = self.project_kpar_url(&uri, &version)?;

//         if !self
//             .client
//             .head(kpar_project_url.clone())
//             .header("ACCEPT", "application/zip, application/octet-stream")
//             .send()?
//             .status()
//             .is_success()
//         {
//             return Ok(None);
//         }

//         if self.try_ranged {
//             if let Ok(proj) = ReqwestKparRangedProject::new_guess_root(&kpar_project_url) {
//                 return Ok(Some(HTTPProject::HTTPKParProjectRanged(proj)));
//             }
//         }

//         Ok(Some(HTTPProject::HTTPKParProjectDownloaded(
//             ReqwestKparDownloadedProject::new_guess_root(&self.project_kpar_url(&uri, &version)?)
//                 .expect("internal IO error"),
//         )))
//     }
// }

impl HTTPEnvironmentAsync {
    pub fn root_url(&self) -> url::Url {
        let mut result = self.base_url.clone();

        if self.base_url.path() != "" {
            result
        } else {
            result.set_path("/");

            result
        }
    }

    pub fn entries_url(&self) -> Result<url::Url, HTTPEnvironmentError> {
        Ok(self.root_url().join(ENTRIES_PATH)?)
    }

    pub fn get_entries_request(
        &self,
    ) -> Result<reqwest_middleware::RequestBuilder, HTTPEnvironmentError> {
        Ok(self.client.get(self.entries_url()?))
    }

    pub fn iri_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
        let mut result = self.root_url();

        for mut component in path_encode_uri(iri) {
            component.push('/');
            result = result.join(&component)?;
        }

        Ok(result)
    }

    pub fn versions_url<S: AsRef<str>>(&self, iri: S) -> Result<url::Url, HTTPEnvironmentError> {
        Ok(self.iri_url(iri)?.join(VERSIONS_PATH)?)
    }

    pub fn get_versions_request<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<reqwest_middleware::RequestBuilder, HTTPEnvironmentError> {
        Ok(self.client.get(self.iri_url(iri)?.join(VERSIONS_PATH)?))
    }

    pub fn project_kpar_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPEnvironmentError> {
        Ok(self
            .iri_url(iri)?
            .join(&format!("{}.kpar", version.as_ref()))?)
    }

    pub fn project_src_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPEnvironmentError> {
        Ok(self
            .iri_url(iri)?
            .join(&format!("{}.kpar/", version.as_ref()))?)
    }

    async fn try_get_project_src<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProjectAsync>, HTTPEnvironmentError> {
        let src_project_url = self.project_src_url(uri, version)?.join(".project.json")?;

        if !self
            .client
            .head(src_project_url.clone())
            .header("ACCEPT", "application/json, text/plain")
            .send()
            .await?
            .status()
            .is_success()
        {
            return Ok(None);
        }

        Ok(Some(HTTPProjectAsync::HTTPSrcProject(
            ReqwestSrcProjectAsync {
                client: self.client.clone(),
                url: src_project_url,
            },
        )))
    }

    async fn try_get_project_kpar<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProjectAsync>, HTTPEnvironmentError> {
        let kpar_project_url = self.project_kpar_url(&uri, &version)?;

        if !self
            .client
            .head(kpar_project_url.clone())
            .header("ACCEPT", "application/zip, application/octet-stream")
            .send()
            .await?
            .status()
            .is_success()
        {
            return Ok(None);
        }

        Ok(Some(HTTPProjectAsync::HTTPKParProjectDownloaded(
            ReqwestKparDownloadedProject::new_guess_root(&self.project_kpar_url(&uri, &version)?)
                .expect("internal IO error")
                .to_async(),
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

impl<I: Stream + std::marker::Unpin> Stream for Optionally<I> {
    type Item = I::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
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

type HTTPLinesIter = std::iter::Map<
    std::io::Lines<BufReader<reqwest::blocking::Response>>,
    fn(Result<String, std::io::Error>) -> Result<String, HTTPEnvironmentError>,
>;

// impl ReadEnvironment for HTTPEnvironment {
//     type ReadError = HTTPEnvironmentError;

//     type UriIter = OptionalIter<HTTPLinesIter>;

//     fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
//         let response = self.get_entries_request()?.send()?;

//         let inner: std::option::Option<HTTPLinesIter> = if response.status().is_success() {
//             Some(
//                 BufReader::new(response)
//                     .lines()
//                     .map(|line| Ok(line?.trim().to_string())),
//             )
//         } else {
//             None
//         };

//         Ok(OptionalIter { inner })
//     }

//     type VersionIter = OptionalIter<HTTPLinesIter>;

//     fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
//         let response = self.get_versions_request(uri)?.send()?;

//         let inner: Option<HTTPLinesIter> = if response.status().is_success() {
//             Some(
//                 BufReader::new(response)
//                     .lines()
//                     .map(|line| Ok(line?.trim().to_string())),
//             )
//         } else {
//             None
//         };

//         Ok(OptionalIter { inner })
//     }

//     type InterchangeProjectRead = HTTPProject;

//     fn get_project<S: AsRef<str>, T: AsRef<str>>(
//         &self,
//         uri: S,
//         version: T,
//     ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
//         if self.prefer_src {
//             if let Some(proj) = self.try_get_project_src(&uri, &version)? {
//                 return Ok(proj);
//             } else if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
//                 return Ok(proj);
//             } else {
//                 return Err(HTTPEnvironmentError::InvalidURL(
//                     self.project_kpar_url(&uri, &version)?,
//                 ));
//             }
//         }

//         if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
//             Ok(proj)
//         } else if let Some(proj) = self.try_get_project_src(&uri, &version)? {
//             Ok(proj)
//         } else {
//             Err(HTTPEnvironmentError::InvalidURL(
//                 self.project_kpar_url(&uri, &version)?,
//             ))
//         }
//     }
// }

fn trim_line<E>(line: Result<String, E>) -> Result<String, E> {
    Ok(line?.trim().to_string())
}

impl ReadEnvironmentAsync for HTTPEnvironmentAsync {
    type ReadError = HTTPEnvironmentError;

    // This can be made more concrete, but the type is humongous
    type UriStream = Optionally<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<std::string::String, HTTPEnvironmentError>>
                    + std::marker::Send,
            >,
        >,
    >;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let response = self.get_entries_request()?.send().await?;

        let inner = if response.status().is_success() {
            Some(
                response
                    .bytes_stream()
                    .map_err(std::io::Error::other)
                    .into_async_read()
                    .lines()
                    .map(trim_line)
                    .map_err(HTTPEnvironmentError::IOError)
                    .boxed(),
            )
        } else {
            None
        };

        Ok(Optionally { inner })
    }

    type VersionStream = Optionally<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<std::string::String, HTTPEnvironmentError>>
                    + std::marker::Send,
            >,
        >,
    >;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        let response = self.get_versions_request(uri)?.send().await?;

        let inner = if response.status().is_success() {
            Some(
                response
                    .bytes_stream()
                    .map_err(std::io::Error::other)
                    .into_async_read()
                    .lines()
                    .map(trim_line)
                    .map_err(HTTPEnvironmentError::IOError)
                    .boxed(),
            )
        } else {
            None
        };

        Ok(Optionally { inner })
    }

    type InterchangeProjectRead = HTTPProjectAsync;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        if self.prefer_src {
            if let Some(proj) = self.try_get_project_src(&uri, &version).await? {
                return Ok(proj);
            } else if let Some(proj) = self.try_get_project_kpar(&uri, &version).await? {
                return Ok(proj);
            } else {
                return Err(HTTPEnvironmentError::InvalidURL(
                    self.project_kpar_url(&uri, &version)?,
                ));
            }
        }

        if let Some(proj) = self.try_get_project_kpar(&uri, &version).await? {
            Ok(proj)
        } else if let Some(proj) = self.try_get_project_src(&uri, &version).await? {
            Ok(proj)
        } else {
            Err(HTTPEnvironmentError::InvalidURL(
                self.project_kpar_url(&uri, &version)?,
            ))
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::{
        env::{ReadEnvironment, ReadEnvironmentAsync},
        resolve::reqwest_http::HTTPProjectAsync,
    };

    #[test]
    fn test_uri_examples() -> Result<(), Box<dyn std::error::Error>> {
        let env = super::HTTPEnvironmentAsync {
            client: reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build(),
            base_url: url::Url::parse("https://www.example.com/a/b")?,
            prefer_src: true,
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
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
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
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
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
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
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
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
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
            //try_ranged: false,
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
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
