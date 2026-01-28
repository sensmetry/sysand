// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{io, marker::Send, pin::Pin, sync::Arc};

use futures::{TryStreamExt, join};
use reqwest_middleware::ClientWithMiddleware;
use thiserror::Error;
/// This module implements accessing interchanged projects stored remotely over HTTP.
/// It is currently written using the blocking Reqwest client. Once sysand functionality
/// has stabilised it will be refactored to use the async interface and allow reqwest_middleware.
/// This will enable middleware (such as caching) as well as using reqwest also in WASM.
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
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
pub struct ReqwestSrcProjectAsync<Pol> {
    /// (reqwest) HTTP client to use for GET requests
    pub client: reqwest_middleware::ClientWithMiddleware, // Internally an Arc
    /// Base-url of the project
    pub url: reqwest::Url,
    pub auth_policy: Arc<Pol>,
}

impl<Pol> ReqwestSrcProjectAsync<Pol> {
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

    // pub fn head_info(&self) -> reqwest_middleware::RequestBuilder {
    //     self.client
    //         .head(self.info_url())
    //         .header(reqwest::header::ACCEPT, "application/json")
    // }

    // pub fn head_meta(&self) -> reqwest_middleware::RequestBuilder {
    //     self.client
    //         .head(self.meta_url())
    //         .header(reqwest::header::ACCEPT, "application/json")
    // }

    // pub fn get_info(&self) -> reqwest_middleware::RequestBuilder {
    //     self.client
    //         .get(self.info_url())
    //         .header(reqwest::header::ACCEPT, "application/json")
    // }

    // pub fn get_meta(&self) -> reqwest_middleware::RequestBuilder {
    //     self.client
    //         .get(self.meta_url())
    //         .header(reqwest::header::ACCEPT, "application/json")
    // }

    pub fn reqwest_src<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> reqwest_middleware::RequestBuilder {
        self.client.get(self.src_url(path))
    }
}

#[derive(Error, Debug)]
pub enum ReqwestSrcError {
    #[error("HTTP request to `{0}` failed: {1}")]
    Reqwest(String, reqwest_middleware::Error),
    #[error("failed to decode response body from HTTP request to `{0}`: {1}")]
    ResponseDecode(String, reqwest::Error),
    #[error("HTTP request to\n  `{0}`\n  returned malformed data: {1}")]
    Deserialize(String, serde_json::Error),
    #[error("HTTP request to `{0}` returned unexpected status code {1}")]
    BadStatus(Box<str>, reqwest::StatusCode),
}

impl<Pol: HTTPAuthentication> ProjectReadAsync for ReqwestSrcProjectAsync<Pol> {
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
        let this_url = self.info_url();
        let info_resp = self
            .auth_policy
            .with_authentication(&self.client, &move |client| client.get(this_url.clone()))
            .await
            .map_err(|e| ReqwestSrcError::Reqwest(self.info_url().into(), e))?;

        Ok(if info_resp.status().is_success() {
            let rep = info_resp
                .text()
                .await
                .map_err(|e| ReqwestSrcError::Reqwest(self.info_url().into(), e.into()))?;
            Some(serde_json::from_str(&rep).map_err(|e| ReqwestSrcError::Deserialize(rep, e))?)
        } else {
            None
        })
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        let this_url = self.meta_url();
        let meta_resp = self
            .auth_policy
            .with_authentication(&self.client, &move |client| client.get(this_url.clone()))
            .await
            .map_err(|e| ReqwestSrcError::Reqwest(self.meta_url().into(), e))?;

        Ok(if meta_resp.status().is_success() {
            let rep = meta_resp
                .text()
                .await
                .map_err(|e| ReqwestSrcError::Reqwest(self.meta_url().into(), e.into()))?;
            Some(serde_json::from_str(&rep).map_err(|e| ReqwestSrcError::Deserialize(rep, e))?)
        } else {
            None
        })
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
            .reqwest_src(&path)
            .send()
            .await
            .map_err(|e| ReqwestSrcError::Reqwest(self.src_url(&path).into(), e))?;

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
        let info_url = self.info_url();
        let info_request = move |client: &ClientWithMiddleware| client.head(info_url.clone());
        let info_resp = self
            .auth_policy
            .with_authentication(&self.client, &info_request);

        let meta_url = self.meta_url();
        let var_name = move |client: &ClientWithMiddleware| client.head(meta_url.clone());
        let meta_resp = self
            .auth_policy
            .with_authentication(&self.client, &var_name);

        match join!(info_resp, meta_resp) {
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

    use crate::{
        auth::Unauthenticated,
        project::{ProjectRead, ProjectReadAsync, reqwest_src::ReqwestSrcProjectAsync},
    };

    #[test]
    fn empty_remote_definitely_invalid_http_src() -> Result<(), Box<dyn std::error::Error>> {
        let server = mockito::Server::new();

        let url = reqwest::Url::parse(&server.url()).unwrap();

        let client =
            reqwest_middleware::ClientBuilder::new(reqwest::ClientBuilder::new().build().unwrap())
                .build();

        let project = ReqwestSrcProjectAsync {
            client,
            url,
            auth_policy: Arc::new(Unauthenticated {}),
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
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

        let project = ReqwestSrcProjectAsync {
            client,
            url,
            auth_policy: Arc::new(Unauthenticated {}),
        }
        .to_tokio_sync(Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
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

        let Err(super::ReqwestSrcError::BadStatus(..)) =
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
