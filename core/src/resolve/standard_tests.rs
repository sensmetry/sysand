// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The standard resolver's index slot over both transports.

use std::sync::Arc;

use camino_tempfile::tempdir;
use fluent_uri::Iri;

use crate::{
    auth::Unauthenticated,
    index_location::IndexSource,
    project::{ProjectRead as _, local_kpar::test_support::write_kpar},
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead as _,
        net_utils::{self, create_reqwest_client},
    },
};

/// Indexes are a union: every configured index is asked, so a folder next to
/// an HTTP index adds candidates and the HTTP index is still contacted. A
/// remote that does not hold the project (its `versions.json` is 404)
/// contributes nothing and the folder's archive is the resolution.
#[test]
fn standard_index_resolver_unions_remote_and_filesystem() {
    let cwd = tempdir().unwrap();
    let folder = cwd.path().join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    write_kpar(&folder.join("dep.kpar"), Some("mock"), "dep", "0.1.0");

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
        crate::cancel::CancellationToken::new(),
    );

    let resolution = ResolutionInfo::iri(Iri::parse("pkg:sysand/mock/dep").unwrap().into());
    let ResolutionOutcome::Resolved(projects) = resolver.resolve_read(&resolution).unwrap() else {
        panic!("the folder holds the project")
    };
    let projects: Vec<_> = projects.into_iter().collect::<Result<_, _>>().unwrap();
    assert_eq!(projects.len(), 1);
    let definition = projects[0].get_project().unwrap().unwrap();
    assert_eq!(
        (definition.name(), definition.version().as_str()),
        ("dep", "0.1.0")
    );

    discovery.assert();
    versions.assert();
}

/// A filesystem-only configuration opens no socket: with every proxy variable
/// pointing at a closed port, the folder still resolves. An HTTP index under
/// the same environment fails (the control).
#[test]
fn filesystem_only_resolution_needs_no_network() {
    let cwd = tempdir().unwrap();
    let folder = cwd.path().join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    write_kpar(&folder.join("dep.kpar"), Some("mock"), "dep", "0.1.0");

    let runtime = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    );
    // A client whose every request goes to a closed port.
    net_utils::install_default_crypto_provider();
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
        crate::cancel::CancellationToken::new(),
    );
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
        crate::cancel::CancellationToken::new(),
    );
    resolver
        .resolve_read(&resolution)
        .expect_err("the proxy sentinel must break the HTTP index");
}
