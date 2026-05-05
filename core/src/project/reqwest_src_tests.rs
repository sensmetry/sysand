// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{io::Read, sync::Arc};

use reqwest::header;
use typed_path::Utf8UnixPath;

use crate::{
    auth::Unauthenticated,
    project::{ProjectRead, ProjectReadAsync, reqwest_src::ReqwestSrcProjectAsync},
    resolve::net_utils::create_reqwest_client,
};

#[test]
fn empty_remote_definitely_invalid_http_src() -> Result<(), Box<dyn std::error::Error>> {
    let server = mockito::Server::new();

    let url = reqwest::Url::parse(&server.url()).unwrap();

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
    let mut server = mockito::Server::new();

    //let host = server.host_with_port();
    let url = reqwest::Url::parse(&server.url()).unwrap();

    let info_mock = server
        .mock("GET", "/.project.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"name":"test_basic_project_urls","version":"1.2.3","usage":[]}"#)
        .match_request(|r| r.has_header(header::USER_AGENT))
        .expect(1)
        .create();

    let meta_mock = server
        .mock("GET", "/.meta.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"index":{},"created":"0000-00-00T00:00:00.123456789Z"}"#)
        .match_request(|r| r.has_header(header::USER_AGENT))
        .expect(1)
        .create();

    let src = "package 'Mekanïk Kommandöh';";

    let src_mock = server
        .mock("GET", "/Mekan%C3%AFk/Kommand%C3%B6h.sysml")
        .with_status(200)
        .with_header("content-type", "text/plain")
        .with_body(src)
        .match_request(|r| r.has_header(header::USER_AGENT))
        .expect(1)
        .create();

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

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!()
    };

    assert_eq!(info.name, "test_basic_project_urls");
    assert_eq!(meta.created, "0000-00-00T00:00:00.123456789Z");

    let mut src_buf = String::new();
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
