// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::io::{BufRead, BufReader};

use sha2::Sha256;
use thiserror::Error;

use crate::{
    env::{
        ReadEnvironment,
        local_directory::{ENTRIES_PATH, VERSIONS_PATH},
        segment_uri_generic,
    },
    project::{
        reqwest_kpar_download::ReqwestKparDownloadedProject,
        reqwest_kpar_ranged::ReqwestKparRangedProject, reqwest_src::ReqwestSrcProject,
    },
    resolve::reqwest_http::HTTPProject,
};

#[derive(Debug)]
pub struct HTTPEnvironment {
    pub client: reqwest::blocking::Client,
    pub base_url: reqwest::Url,
    pub prefer_src: bool,
    pub try_ranged: bool,
}

#[derive(Error, Debug)]
pub enum HTTPEnvironmentError {
    #[error("failed to extend URL '{0}' with path '{1}': {2}")]
    JoinURL(Box<str>, String, url::ParseError),
    #[error("error making an HTTP request to '{0}':\n{1}")]
    HTTPRequest(Box<str>, reqwest::Error),
    #[error("failed to get project '{0}', version '{1}' in source or kpar format")]
    InvalidURL(Box<str>, Box<str>),
    #[error("failed to read HTTP response: {0}")]
    HttpIo(std::io::Error),
}

pub fn path_encode_uri<S: AsRef<str>>(uri: S) -> std::vec::IntoIter<std::string::String> {
    segment_uri_generic::<S, Sha256>(uri)
}

impl HTTPEnvironment {
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

    pub fn get_entries_request(
        &self,
    ) -> Result<reqwest::blocking::RequestBuilder, HTTPEnvironmentError> {
        Ok(self.client.get(self.entries_url()?))
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

    pub fn get_versions_request<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<reqwest::blocking::RequestBuilder, HTTPEnvironmentError> {
        Ok(self.client.get(self.versions_url(iri)?))
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

    fn try_get_project_src<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProject>, HTTPEnvironmentError> {
        let project_url = self.project_src_url(uri, version)?;
        let src_project_url = Self::url_join(&project_url, ".project.json")?;

        if !self
            .client
            .head(src_project_url.clone())
            .header("ACCEPT", "application/json, text/plain")
            .send()
            .map_err(|e| HTTPEnvironmentError::HTTPRequest(src_project_url.as_str().into(), e))?
            .status()
            .is_success()
        {
            return Ok(None);
        }

        Ok(Some(HTTPProject::HTTPSrcProject(ReqwestSrcProject {
            client: self.client.clone(),
            url: src_project_url,
        })))
    }

    fn try_get_project_kpar<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProject>, HTTPEnvironmentError> {
        let kpar_project_url = self.project_kpar_url(&uri, &version)?;

        if !self
            .client
            .head(kpar_project_url.clone())
            .header("ACCEPT", "application/zip, application/octet-stream")
            .send()
            .map_err(|e| HTTPEnvironmentError::HTTPRequest(kpar_project_url.as_str().into(), e))?
            .status()
            .is_success()
        {
            return Ok(None);
        }

        if self.try_ranged {
            if let Ok(proj) = ReqwestKparRangedProject::new_guess_root(&kpar_project_url) {
                return Ok(Some(HTTPProject::HTTPKParProjectRanged(proj)));
            }
        }

        Ok(Some(HTTPProject::HTTPKParProjectDownloaded(
            ReqwestKparDownloadedProject::new_guess_root(&self.project_kpar_url(&uri, &version)?)
                .expect("internal IO error"),
        )))
    }
}

#[derive(Debug)]
pub struct OptionalIter<I> {
    inner: Option<I>,
}

impl<I: Iterator> Iterator for OptionalIter<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(inner) = &mut self.inner {
            inner.next()
        } else {
            None
        }
    }
}

type HTTPLinesIter = std::iter::Map<
    std::io::Lines<BufReader<reqwest::blocking::Response>>,
    fn(Result<String, std::io::Error>) -> Result<String, HTTPEnvironmentError>,
>;

impl ReadEnvironment for HTTPEnvironment {
    type ReadError = HTTPEnvironmentError;

    type UriIter = OptionalIter<HTTPLinesIter>;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let response = self.get_entries_request()?.send().map_err(|e| {
            HTTPEnvironmentError::HTTPRequest(self.entries_url().unwrap().as_str().into(), e)
        })?;

        let inner: Option<HTTPLinesIter> = if response.status().is_success() {
            Some(BufReader::new(response).lines().map(|line| {
                Ok(line
                    .map_err(HTTPEnvironmentError::HttpIo)?
                    .trim()
                    .to_string())
            }))
        } else {
            None
        };

        Ok(OptionalIter { inner })
    }

    type VersionIter = OptionalIter<HTTPLinesIter>;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let response = self.get_versions_request(&uri)?.send().map_err(|e| {
            HTTPEnvironmentError::HTTPRequest(self.versions_url(uri).unwrap().as_str().into(), e)
        })?;

        let inner: Option<HTTPLinesIter> = if response.status().is_success() {
            Some(BufReader::new(response).lines().map(|line| {
                Ok(line
                    .map_err(HTTPEnvironmentError::HttpIo)?
                    .trim()
                    .to_string())
            }))
        } else {
            None
        };

        Ok(OptionalIter { inner })
    }

    type InterchangeProjectRead = HTTPProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        if self.prefer_src {
            if let Some(proj) = self.try_get_project_src(&uri, &version)? {
                Ok(proj)
            } else if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
                Ok(proj)
            } else {
                Err(HTTPEnvironmentError::InvalidURL(
                    uri.as_ref().into(),
                    version.as_ref().into(),
                ))
            }
        } else if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
            Ok(proj)
        } else if let Some(proj) = self.try_get_project_src(&uri, &version)? {
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
    use crate::{env::ReadEnvironment, resolve::reqwest_http::HTTPProject};

    #[test]
    fn test_uri_examples() -> Result<(), Box<dyn std::error::Error>> {
        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse("https://www.example.com/a/b")?,
            prefer_src: true,
            try_ranged: false,
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

        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            try_ranged: false,
        };

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

        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            try_ranged: false,
        };

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

        let HTTPProject::HTTPKParProjectDownloaded(_) = project else {
            panic!("Expected to resolve to KPar project");
        };

        kpar_mock.assert();

        Ok(())
    }

    #[test]
    fn test_src_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse(&host)?,
            prefer_src: false,
            try_ranged: false,
        };

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

        let HTTPProject::HTTPSrcProject(_) = project else {
            panic!("Expected to resolve to src project");
        };

        src_mock.assert();

        Ok(())
    }

    #[test]
    fn test_kpar_preference() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        let host = server.url();

        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse(&host)?,
            prefer_src: false,
            try_ranged: false,
        };

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

        let HTTPProject::HTTPKParProjectDownloaded(_) = project else {
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

        let env = super::HTTPEnvironment {
            client: reqwest::blocking::Client::new(),
            base_url: url::Url::parse(&host)?,
            prefer_src: true,
            try_ranged: false,
        };

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

        let HTTPProject::HTTPSrcProject(_) = project else {
            panic!("Expected to resolve to src project");
        };

        src_mock.assert();
        kpar_mock.assert();

        Ok(())
    }
}
