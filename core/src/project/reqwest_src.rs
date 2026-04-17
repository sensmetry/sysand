// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! This module implements accessing interchanged projects stored remotely over HTTP.

use std::{io, marker::Send, pin::Pin, sync::Arc};

use futures::{TryStreamExt, join};
use thiserror::Error;
use typed_path::Utf8UnixPath;

use crate::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectReadAsync,
    resolve::net_utils::{json_get_request, json_head_request, text_get_request},
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
pub struct ReqwestSrcProjectAsync<Policy> {
    /// (reqwest) HTTP client to use for GET requests
    pub client: reqwest_middleware::ClientWithMiddleware, // Internally an Arc
    /// Base-url of the project
    pub url: reqwest::Url,
    pub auth_policy: Arc<Policy>,
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

    // pub fn reqwest_src<P: AsRef<Utf8UnixPath>>(
    //     &self,
    //     path: P,
    // ) -> reqwest_middleware::RequestBuilder {
    //     self.client.get(self.src_url(path))
    // }
}

#[derive(Error, Debug)]
pub enum ReqwestSrcError {
    #[error("error making an HTTP request:\n{0:#?}")]
    ReqwestMiddleware(reqwest_middleware::Error),
    #[error("error making an HTTP request:\n{0:#?}")]
    Reqwest(reqwest::Error),
    #[error("HTTP request to\n  `{0}`\n  returned malformed data: {1}")]
    Deserialize(String, serde_json::Error),
    #[error("HTTP request to `{0}` returned unexpected status code {1}")]
    BadStatus(Box<str>, reqwest::StatusCode),
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
        let (info, meta) = join!(self.get_info_async(), self.get_meta_async());

        Ok((info?, meta?))
    }

    async fn get_info_async(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        let info_resp = self
            .auth_policy
            .with_authentication(&self.client, &json_get_request(self.info_url()))
            .await
            .map_err(ReqwestSrcError::ReqwestMiddleware)?;

        Ok(if info_resp.status().is_success() {
            let rep = info_resp.text().await.map_err(ReqwestSrcError::Reqwest)?;
            Some(serde_json::from_str(&rep).map_err(|e| ReqwestSrcError::Deserialize(rep, e))?)
        } else {
            None
        })
    }

    async fn get_meta_async(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        let meta_resp = self
            .auth_policy
            .with_authentication(&self.client, &json_get_request(self.meta_url()))
            .await
            .map_err(ReqwestSrcError::ReqwestMiddleware)?;

        Ok(if meta_resp.status().is_success() {
            let rep = meta_resp.text().await.map_err(ReqwestSrcError::Reqwest)?;
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
        Ok(vec![Source::RemoteSrc {
            remote_src: self.url.to_string(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;
    use std::{io::Read, sync::Arc};

    use reqwest::header;
    use typed_path::Utf8UnixPath;

    use crate::{
        auth::Unauthenticated,
        project::{ProjectRead, ProjectReadAsync, reqwest_src::ReqwestSrcProjectAsync},
        resolve::net_utils::create_reqwest_client,
        test_utils::{Created, ProjectMock},
    };

    #[test]
    fn empty_remote_definitely_invalid_http_src() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();

        let url = reqwest::Url::parse(&server.base_url()).unwrap();

        let client = create_reqwest_client()?;

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
        let file_path = "Mekanïk/Kommandöh.sysml";
        let file_src = "package 'Mekanïk Kommandöh';";

        let project_mock = ProjectMock::builder(
            "test_basic_project_urls",
            "1.2.3",
            Created::Custom("0000-00-00T00:00:00.123456789Z".into()),
        )
        .with_files([(file_path, file_src)], true, true)
        .build();
        let server = MockServer::start();
        let url = reqwest::Url::parse(&server.base_url()).unwrap();
        let mocks = project_mock.add_files_to_server(
            &server,
            |when| when.header_exists(header::USER_AGENT.as_str()),
            |then| then,
        );

        let client = create_reqwest_client()?;

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

        // let (Some(info), Some(meta)) = project.get_project()?;

        let downloaded_project = project.get_project()?;
        let (Some(info), Some(meta)) = downloaded_project else {
            panic!(
                "Info and meta must not be None. Info: {:?}, Meta: {:?}",
                downloaded_project.0, downloaded_project.1
            )
        };

        assert_eq!(info.name, "test_basic_project_urls");
        assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");

        let mut src_buf = String::new();
        project
            .read_source(Utf8UnixPath::new(file_path).to_path_buf())?
            .read_to_string(&mut src_buf)?;

        assert_eq!(file_src, src_buf);

        let Err(super::ReqwestSrcError::BadStatus(..)) =
            project.read_source(Utf8UnixPath::new("Mekanik/Kommandoh.sysml").to_path_buf())
        else {
            panic!();
        };

        for (_path, mock) in mocks.head.iter() {
            mock.assert_calls(0);
        }
        for (_path, mock) in mocks.get.iter() {
            mock.assert_calls(1);
        }

        Ok(())
    }
}
