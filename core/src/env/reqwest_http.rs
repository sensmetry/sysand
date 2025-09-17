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
    #[error("{0}")]
    URLError(#[from] url::ParseError),
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

    pub fn entries_url(&self) -> Result<url::Url, HTTPEnvironmentError> {
        Ok(self.root_url().join(ENTRIES_PATH)?)
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
    ) -> Result<reqwest::blocking::RequestBuilder, HTTPEnvironmentError> {
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

    fn try_get_project_src<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Option<HTTPProject>, HTTPEnvironmentError> {
        let src_project_url = self.project_src_url(uri, version)?.join(".project.json")?;

        if !self
            .client
            .head(src_project_url.clone())
            .header("ACCEPT", "application/json, text/plain")
            .send()?
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
            .send()?
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

impl ReadEnvironment for HTTPEnvironment {
    type ReadError = HTTPEnvironmentError;

    type UriIter = std::iter::Map<
        std::io::Lines<BufReader<reqwest::blocking::Response>>,
        fn(Result<String, std::io::Error>) -> Result<String, Self::ReadError>,
    >;

    fn uris(&self) -> Result<Self::UriIter, Self::ReadError> {
        let response = self.get_entries_request()?.send()?;

        Ok(BufReader::new(response)
            .lines()
            .map(|line| Ok(line?.trim().to_string())))
    }

    type VersionIter = std::iter::Map<
        std::io::Lines<BufReader<reqwest::blocking::Response>>,
        fn(Result<String, std::io::Error>) -> Result<String, Self::ReadError>,
    >;

    fn versions<S: AsRef<str>>(&self, uri: S) -> Result<Self::VersionIter, Self::ReadError> {
        let response = self.get_versions_request(uri)?.send()?;

        Ok(BufReader::new(response)
            .lines()
            .map(|line| Ok(line?.trim().to_string())))
    }

    type InterchangeProjectRead = HTTPProject;

    fn get_project<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        if self.prefer_src {
            if let Some(proj) = self.try_get_project_src(&uri, &version)? {
                return Ok(proj);
            } else if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
                return Ok(proj);
            } else {
                return Err(HTTPEnvironmentError::InvalidURL(
                    self.project_kpar_url(&uri, &version)?,
                ));
            }
        }

        if let Some(proj) = self.try_get_project_kpar(&uri, &version)? {
            Ok(proj)
        } else if let Some(proj) = self.try_get_project_src(&uri, &version)? {
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
