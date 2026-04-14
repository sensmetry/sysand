// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{Read, Write as _},
    sync::Arc,
};

use crate::{
    auth::Unauthenticated,
    project::{ProjectRead, ProjectReadAsync},
    resolve::net_utils::create_reqwest_client,
};

#[test]
fn test_basic_download_request() -> Result<(), Box<dyn std::error::Error>> {
    let buf = {
        let mut cursor = std::io::Cursor::new(vec![]);
        let mut zip = zip::ZipWriter::new(&mut cursor);

        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);

        zip.start_file("some_root_dir/.project.json", options)?;
        zip.write_all(br#"{"name":"test_basic_download_request","version":"1.2.3","usage":[]}"#)?;
        zip.start_file("some_root_dir/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"123"}"#)?;
        zip.start_file("some_root_dir/test.sysml", options)?;
        zip.write_all(br#"package Test;"#)?;

        zip.finish().unwrap();

        cursor.flush()?;
        cursor.into_inner()
    };

    let mut server = mockito::Server::new();

    //let host = server.host_with_port();
    let url = reqwest::Url::parse(&server.url()).unwrap();

    let get_kpar = server
        .mock("GET", "/test_basic_download_request.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&buf)
        .create();

    let project = super::ReqwestKparDownloadedProject::new_guess_root(
        format!("{}test_basic_download_request.kpar", url,),
        create_reqwest_client()?,
        Arc::new(Unauthenticated {}),
    )?
    .to_tokio_sync(Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    ));

    let (Some(info), Some(meta)) = project.get_project()? else {
        panic!()
    };

    assert_eq!(info.name, "test_basic_download_request");
    assert_eq!(meta.created, "123");

    let mut src = String::new();
    project
        .read_source("test.sysml")?
        .read_to_string(&mut src)?;

    assert_eq!(src, "package Test;");

    get_kpar.assert();

    Ok(())
}
