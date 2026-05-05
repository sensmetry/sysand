// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    io::{Read, Write as _},
    sync::Arc,
};

use crate::{
    auth::Unauthenticated,
    context::ProjectContext,
    lock::Source,
    project::{ProjectRead, ProjectReadAsync, reqwest_kpar_download::ReqwestKparDownloadedError},
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
        None,
        None,
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

/// Two concurrent `ensure_downloaded_verified` calls on the same
/// project must fan in to a single download. Without the per-project
/// download lock, both tasks open the destination archive path (which
/// `wrapfs::File::create` truncates), and interleave writes — each
/// task's hasher passes against its own stream but the file on disk is
/// corrupt.
///
/// This test serializes through a real `reqwest`/`mockito` round-trip, so
/// both futures race through the same code paths a real caller would hit.
/// We assert:
///   - both futures resolve `Ok(())`,
///   - the server observed exactly one kpar fetch (`expect(1)`),
///   - the archive on disk parses and exposes the expected project —
///     i.e. the bytes weren't interleaved.
#[test]
fn test_concurrent_downloads_fan_in_to_single_fetch() -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest as _, Sha256};

    let kpar_bytes = {
        let mut cursor = std::io::Cursor::new(vec![]);
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file("root/.project.json", options)?;
        zip.write_all(br#"{"name":"concurrent","version":"1.0.0","usage":[]}"#)?;
        zip.start_file("root/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"x"}"#)?;
        zip.finish().unwrap();
        cursor.flush()?;
        cursor.into_inner()
    };
    let expected_digest = format!("{:x}", Sha256::digest(&kpar_bytes));

    let mut server = mockito::Server::new();
    let url = reqwest::Url::parse(&server.url())?;

    // `expect(1)` pins the invariant: exactly one request reaches the
    // server, even under racing callers.
    let get_kpar = server
        .mock("GET", "/concurrent.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&kpar_bytes)
        .expect(1)
        .create();

    let project = Arc::new(super::ReqwestKparDownloadedProject::new_guess_root(
        format!("{url}concurrent.kpar"),
        create_reqwest_client()?,
        Arc::new(Unauthenticated {}),
        Some(expected_digest),
        None,
    )?);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Two futures on the same runtime enter `ensure_downloaded_verified`
    // together. `OnceCell` must fan them into one download; without that,
    // both would proceed into the direct-to-destination write path.
    let p1 = Arc::clone(&project);
    let p2 = Arc::clone(&project);
    let (r1, r2) = runtime.block_on(async move {
        futures::future::join(
            async move { p1.ensure_downloaded_verified().await },
            async move { p2.ensure_downloaded_verified().await },
        )
        .await
    });
    r1?;
    r2?;

    get_kpar.assert();

    // The installed archive must parse — a corrupted interleaved write
    // would fail here or return garbage.
    let (Some(info), Some(_meta)) = project.inner.get_project()? else {
        panic!("installed archive failed to expose project");
    };
    assert_eq!(info.name, "concurrent");

    Ok(())
}

#[test]
fn test_expected_size_mismatch_rejects_download() -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest as _, Sha256};

    let kpar_bytes = {
        let mut cursor = std::io::Cursor::new(vec![]);
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        zip.start_file("root/.project.json", options)?;
        zip.write_all(br#"{"name":"size-mismatch","version":"1.0.0","usage":[]}"#)?;
        zip.start_file("root/.meta.json", options)?;
        zip.write_all(br#"{"index":{},"created":"x"}"#)?;
        zip.finish().unwrap();
        cursor.flush()?;
        cursor.into_inner()
    };
    let expected_digest = format!("{:x}", Sha256::digest(&kpar_bytes));
    let wrong_size = kpar_bytes.len() as u64 - 1;

    let mut server = mockito::Server::new();
    let url = reqwest::Url::parse(&server.url())?;

    let get_kpar = server
        .mock("GET", "/size-mismatch.kpar")
        .with_status(200)
        .with_header("content-type", "application/zip")
        .with_body(&kpar_bytes)
        .expect(1)
        .create();

    let project = super::ReqwestKparDownloadedProject::new_guess_root(
        format!("{url}size-mismatch.kpar"),
        create_reqwest_client()?,
        Arc::new(Unauthenticated {}),
        Some(expected_digest),
        Some(std::num::NonZeroU64::new(wrong_size).unwrap()),
    )?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        match project.ensure_downloaded_verified().await {
            Err(ReqwestKparDownloadedError::SizeMismatch {
                expected, actual, ..
            }) => {
                assert_eq!(expected, wrong_size);
                assert_eq!(actual, kpar_bytes.len() as u64);
            }
            other => panic!("expected SizeMismatch for wrong kpar size, got {other:?}"),
        }
    });

    assert!(
        !project.is_downloaded_and_verified(),
        "size mismatch must not be reported as success"
    );
    assert!(
        !project.inner.archive_path.exists(),
        "content-length mismatch must not create an archive file"
    );
    get_kpar.assert();

    Ok(())
}

#[test]
fn test_index_kpar_source_roundtrips_digest_and_size() -> Result<(), Box<dyn std::error::Error>> {
    let index_kpar = "https://example.com/project.kpar";
    let index_kpar_size = std::num::NonZeroU64::new(1234).unwrap();
    let index_kpar_digest = "a".repeat(64);

    let project = super::ReqwestKparDownloadedProject::new_guess_root(
        index_kpar,
        create_reqwest_client()?,
        Arc::new(Unauthenticated {}),
        Some(index_kpar_digest.clone()),
        Some(index_kpar_size),
    )?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let sources = runtime.block_on(project.sources_async(&ProjectContext::default()))?;

    assert_eq!(
        sources,
        vec![Source::IndexKpar {
            index_kpar: index_kpar.to_string(),
            index_kpar_size,
            index_kpar_digest,
        }]
    );

    Ok(())
}
