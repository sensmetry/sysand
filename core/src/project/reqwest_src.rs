// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use futures::{TryStreamExt, join};
use thiserror::Error;
/// This module implements accessing interchanged projects stored remotely over HTTP.
/// It is currently written using the blocking Reqwest client. Once sysand functionality
/// has stabilised it will be refactored to use the async interface and allow reqwest_middleware.
/// This will enable middleware (such as caching) as well as using reqwest also in WASM.
use typed_path::Utf8UnixPath;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectReadAsync,
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
pub struct ReqwestSrcProjectAsync {
    /// (reqwest) HTTP client to use for GET requests
    pub client: reqwest_middleware::ClientWithMiddleware, // Internally an Arc
    /// Base-url of the project
    pub url: reqwest::Url,
}

impl ReqwestSrcProjectAsync {
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

    pub fn head_info(&self) -> reqwest_middleware::RequestBuilder {
        self.client
            .head(self.info_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn head_meta(&self) -> reqwest_middleware::RequestBuilder {
        self.client
            .head(self.meta_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn get_info(&self) -> reqwest_middleware::RequestBuilder {
        self.client
            .get(self.info_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn get_meta(&self) -> reqwest_middleware::RequestBuilder {
        self.client
            .get(self.meta_url())
            .header(reqwest::header::ACCEPT, "application/json")
    }

    pub fn reqwest_src<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> reqwest_middleware::RequestBuilder {
        self.client.get(self.src_url(path))
    }
}

#[derive(Error, Debug)]
pub enum ReqwestSrcError {
    #[error("HTTP middleware error: {0}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("HTTP error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Malformed data: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

impl ProjectReadAsync for ReqwestSrcProjectAsync {
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
        let (info, meta) = join!(self.get_info_async(), self.get_meta_async());

        Ok((info?, meta?))
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        let info_resp = self.get_info().send().await?;

        Ok(if info_resp.status().is_success() {
            Some(serde_json::from_str(&info_resp.text().await?)?)
        } else {
            None
        })
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        let meta_resp = self.get_meta().send().await?;

        Ok(if meta_resp.status().is_success() {
            Some(serde_json::from_str(&meta_resp.text().await?)?)
        } else {
            None
        })
    }

    type SourceReader<'a>
        = futures::stream::IntoAsyncRead<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<bytes::Bytes, std::io::Error>>
                    + std::marker::Send,
            >,
        >,
    >
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        use futures::StreamExt as _;

        let resp = self.reqwest_src(&path).send().await?;

        if resp.status().is_success() {
            Ok(resp
                .bytes_stream()
                .map_err(std::io::Error::other)
                .boxed()
                .into_async_read())
        } else {
            Err(ReqwestSrcError::NotFound(format!(
                "path {} not found",
                path.as_ref()
            )))
        }
    }

    async fn is_definitely_invalid_async(&self) -> bool {
        match join!(self.head_info().send(), self.head_meta().send()) {
            (Ok(info_head), Ok(meta_head)) => {
                !info_head.status().is_success() || !meta_head.status().is_success()
            }
            _ => true,
        }
    }

    async fn sources_async(&self) -> Vec<crate::lock::Source> {
        vec![crate::lock::Source::RemoteSrc {
            remote_src: self.url.to_string(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, sync::Arc};

    use typed_path::Utf8UnixPath;

    use crate::project::{ProjectRead, ProjectReadAsync, reqwest_src::ReqwestSrcProjectAsync};

    #[test]
    fn empty_remote_definitely_invalid_http_src() -> Result<(), Box<dyn std::error::Error>> {
        let server = mockito::Server::new();

        let url = reqwest::Url::parse(&server.url()).unwrap();

        let client =
            reqwest_middleware::ClientBuilder::new(reqwest::ClientBuilder::new().build().unwrap())
                .build();

        let project = ReqwestSrcProjectAsync { client, url }.to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
        ));

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

        let client =
            reqwest_middleware::ClientBuilder::new(reqwest::ClientBuilder::new().build().unwrap())
                .build();

        let project = ReqwestSrcProjectAsync { client, url }.to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread().build()?,
        ));

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
