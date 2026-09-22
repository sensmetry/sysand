// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The standard resolver's index slot over both transports
//! (`design/index-protocol.md` "Filesystem indexes").

use std::{io::Write as _, sync::Arc};

use camino::Utf8Path;
use camino_tempfile::tempdir;
use fluent_uri::Iri;

use crate::{
    auth::Unauthenticated,
    index_location::IndexSource,
    project::ProjectRead as _,
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead as _, net_utils::create_reqwest_client,
    },
};

fn write_kpar(path: &Utf8Path, publisher: &str, name: &str, version: &str) {
    let info = format!(r#"{{"name":"{name}","publisher":"{publisher}","version":"{version}"}}"#);
    let meta = r#"{"index":{},"created":"2026-01-01T00:00:00.000000000Z"}"#;
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    zip.start_file(".project.json", options).unwrap();
    zip.write_all(info.as_bytes()).unwrap();
    zip.start_file(".meta.json", options).unwrap();
    zip.write_all(meta.as_bytes()).unwrap();
    zip.finish().unwrap();
}

/// Indexes are a union: every configured index is asked (the sequential
/// resolver queries them together and propagates any error), so a
/// folder next to an HTTP index adds candidates and the HTTP index is
/// still contacted. A remote that does not hold the project (its
/// `versions.json` is 404) contributes nothing and the folder's archive
/// is the resolution.
#[test]
fn standard_index_resolver_unions_remote_and_filesystem() {
    let cwd = tempdir().unwrap();
    let folder = cwd.path().join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    write_kpar(&folder.join("dep.kpar"), "mock", "dep", "0.1.0");

    let mut server = mockito::Server::new();
    let discovery = server
        .mock("GET", "/sysand-index-config.json")
        .with_status(404)
        .expect(1)
        .create();
    let versions = server
        .mock("GET", "/mock/dep/versions.json")
        .with_status(404)
        .expect(1)
        .create();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    let resolver = super::standard_index_resolver(
        create_reqwest_client().unwrap(),
        vec![
            IndexSource::parse(&server.url()).unwrap(),
            IndexSource::Filesystem(folder),
        ],
        runtime,
        Arc::new(Unauthenticated {}),
    )
    .unwrap();

    let resolution = ResolutionInfo::iri(Iri::parse("pkg:sysand/mock/dep").unwrap().into());
    let ResolutionOutcome::Resolved(projects) = resolver.resolve_read(&resolution).unwrap() else {
        panic!("the folder holds the project")
    };
    let projects: Vec<_> = projects.into_iter().collect::<Result<_, _>>().unwrap();
    assert_eq!(projects.len(), 1);
    let info = projects[0].get_info().unwrap().unwrap();
    assert_eq!(
        (info.name.as_str(), info.version.as_str()),
        ("dep", "0.1.0")
    );

    discovery.assert();
    versions.assert();
}

/// A filesystem-only configuration opens no socket: with every proxy
/// variable pointing at a closed port, the folder still resolves. The
/// same environment breaks an HTTP index (the control), so the
/// assertion is about denial, not non-use.
#[test]
fn filesystem_only_resolution_needs_no_network() {
    let cwd = tempdir().unwrap();
    let folder = cwd.path().join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    write_kpar(&folder.join("dep.kpar"), "mock", "dep", "0.1.0");

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    // A client whose every request goes to a closed port.
    let denied = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
        .build()
        .unwrap();
    let denied = reqwest_middleware::ClientBuilder::new(denied).build();

    let resolution = ResolutionInfo::iri(Iri::parse("pkg:sysand/mock/dep").unwrap().into());

    let resolver = super::standard_index_resolver(
        denied.clone(),
        vec![IndexSource::Filesystem(folder)],
        runtime.clone(),
        Arc::new(Unauthenticated {}),
    )
    .unwrap();
    let ResolutionOutcome::Resolved(projects) = resolver.resolve_read(&resolution).unwrap() else {
        panic!("the folder holds the project")
    };
    assert_eq!(projects.into_iter().count(), 1);

    // Control: an HTTP index under the same client cannot be reached.
    let server = mockito::Server::new();
    let resolver = super::standard_index_resolver(
        denied,
        vec![IndexSource::parse(&server.url()).unwrap()],
        runtime,
        Arc::new(Unauthenticated {}),
    )
    .unwrap();
    resolver
        .resolve_read(&resolution)
        .expect_err("the proxy sentinel must break the HTTP index");
}
