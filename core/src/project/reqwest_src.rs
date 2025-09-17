// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

/// This module implements accessing interchanged projects stored remotely over HTTP.
/// It is currently written using the blocking Reqwest client. Once sysand functionality
/// has stabilised it will be refactored to use the async interface and allow reqwest_middleware.
/// This will enable middleware (such as caching) as well as using reqwest also in WASM.
use typed_path::Utf8UnixPath;

use thiserror::Error;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectRead,
};

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
pub struct ReqwestSrcProject {
    /// (reqwest) HTTP client to use for GET requests
    pub client: reqwest::blocking::Client, // Internally an Arc
    /// Base-url of the project
    pub url: reqwest::Url,
}

impl ReqwestSrcProject {
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

    pub fn head_info(&self) -> reqwest::blocking::RequestBuilder {
        self.client
            .head(self.info_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn head_meta(&self) -> reqwest::blocking::RequestBuilder {
        self.client
            .head(self.meta_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn get_info(&self) -> reqwest::blocking::RequestBuilder {
        self.client
            .get(self.info_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn get_meta(&self) -> reqwest::blocking::RequestBuilder {
        self.client
            .get(self.meta_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn reqwest_src<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> reqwest::blocking::RequestBuilder {
        self.client.get(self.src_url(path))
    }
}

#[derive(Error, Debug)]
pub enum ReqwestSrcError {
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Malformed data: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

impl ProjectRead for ReqwestSrcProject {
    type Error = ReqwestSrcError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        let info_resp = self.get_info().send()?;
        let meta_resp = self.get_meta().send()?;

        let info: Option<InterchangeProjectInfoRaw> = if info_resp.status().is_success() {
            Some(serde_json::from_str(&info_resp.text()?)?)
        } else {
            None
        };

        let meta: Option<InterchangeProjectMetadataRaw> = if meta_resp.status().is_success() {
            Some(serde_json::from_str(&meta_resp.text()?)?)
        } else {
            None
        };

        Ok((info, meta))
    }

    type SourceReader<'a>
        = reqwest::blocking::Response
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        let resp = self.reqwest_src(&path).send()?;

        if resp.status().is_success() {
            Ok(resp)
        } else {
            Err(ReqwestSrcError::NotFound(format!(
                "path {} not found",
                path.as_ref()
            )))
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match (self.head_info().send(), self.head_meta().send()) {
            (Ok(info_head), Ok(meta_head)) => {
                !info_head.status().is_success() || !meta_head.status().is_success()
            }
            _ => true,
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::RemoteSrc {
            remote_src: self.url.to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use typed_path::Utf8UnixPath;

    use crate::project::{ProjectRead, reqwest_src::ReqwestSrcProject};

    #[test]
    fn empty_remote_definitely_invalid_http_src() -> Result<(), Box<dyn std::error::Error>> {
        let server = mockito::Server::new();

        let url = reqwest::Url::parse(&server.url()).unwrap();

        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();

        let project = ReqwestSrcProject { client, url };

        assert!(project.is_definitely_invalid());

        Ok(())
    }

    #[test]
    fn test_basic_project_urls_http_src() -> Result<(), Box<dyn std::error::Error>> {
        let mut server = mockito::Server::new();

        //let host = server.host_with_port();
        let url = reqwest::Url::parse(&server.url()).unwrap();

        let info_mock = server
            .mock("GET", "/.project.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"name":"test_basic_project_urls","version":"1.2.3","usage":[]}"#)
            .create();

        let meta_mock = server
            .mock("GET", "/.meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
            .create();

        let src = "package 'Mekanïk Kommandöh';";

        let src_mock = server
            .mock("GET", "/Mekan%C3%AFk/Kommand%C3%B6h.sysml")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body(src)
            .create();

        let client = reqwest::blocking::ClientBuilder::new().build().unwrap();

        let project = ReqwestSrcProject { client, url };

        let (Some(info), Some(meta)) = project.get_project()? else {
            panic!()
        };

        assert_eq!(info.name, "test_basic_project_urls");
        assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");

        let mut src_buf = "".to_string();
        project
            .read_source(Utf8UnixPath::new("Mekanïk/Kommandöh.sysml").to_path_buf())?
            .read_to_string(&mut src_buf)?;

        assert_eq!(src, src_buf);

        let Err(super::ReqwestSrcError::NotFound(_)) =
            project.read_source(Utf8UnixPath::new("Mekanik/Kommandoh.sysml").to_path_buf())
        else {
            panic!();
        };

        info_mock.assert();
        meta_mock.assert();
        src_mock.assert();

        Ok(())
    }
}
