// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Sibling unit tests for `index_entry.rs`.
//!
//! Network-dependent paths (pre-expose `project_digest` check,
//! post-download digest drift, `.project.json`/`.meta.json` fetch error
//! mapping) live in `core/src/env/index_tests.rs` because they need
//! mockito to drive the HTTP side. These tests pin the network-free
//! contracts on the advertised tier — the module docs promise that
//! `version_async`/`usage_async` and, before any download,
//! `checksum_canonical_hex_async` return advertised fields without
//! I/O, and the only way to regress that silently is to add an
//! `fetched_project_async` call into one of those methods. A sibling
//! test is the natural place to lock that contract in.

use std::sync::Arc;

use crate::{
    auth::Unauthenticated,
    env::index::{AdvertisedVersion, Sha256HexDigest},
    model::InterchangeProjectUsageRaw,
    project::{ProjectReadAsync, index_entry::IndexEntryProject},
    purl::PKG_SYSAND_PREFIX,
    resolve::net_utils::create_reqwest_client,
};

/// Construct an `IndexEntryProject` whose advertised tier has known
/// values and whose archive is never downloaded. The returned project
/// exposes `archive.is_downloaded_and_verified() == false`, so
/// advertised-tier reads that honour the "no I/O before download"
/// contract must succeed without touching the network (which the
/// unreachable mock URL would otherwise error on).
fn make_fixture() -> IndexEntryProject<Unauthenticated> {
    // Two distinct 64-hex digests so a test that confuses them fails
    // loudly rather than passing on equality.
    let project_digest = Sha256HexDigest::try_from(
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    )
    .expect("fixture project_digest must be valid sha256 hex");
    let kpar_digest = Sha256HexDigest::try_from(
        "sha256:2222222222222222222222222222222222222222222222222222222222222222",
    )
    .expect("fixture kpar_digest must be valid sha256 hex");

    let advertised = AdvertisedVersion {
        version: semver::Version::parse("1.2.3").unwrap(),
        usage: vec![InterchangeProjectUsageRaw {
            resource: format!("{PKG_SYSAND_PREFIX}acme/widget"),
            version_constraint: Some("^1.0".to_string()),
        }],
        project_digest,
        kpar_size: 42,
        kpar_digest,
        status: crate::env::index::Status::Available,
    };

    // `test.invalid` is reserved by RFC 2606; any accidental fetch
    // fails DNS resolution rather than hitting a live host.
    let kpar_url = reqwest::Url::parse("http://test.invalid/kpar").unwrap();
    let project_json_url = reqwest::Url::parse("http://test.invalid/.project.json").unwrap();
    let meta_json_url = reqwest::Url::parse("http://test.invalid/.meta.json").unwrap();

    IndexEntryProject::new(
        kpar_url,
        project_json_url,
        meta_json_url,
        advertised,
        create_reqwest_client().expect("reqwest client builder succeeds"),
        Arc::new(Unauthenticated {}),
    )
    .expect("IndexEntryProject::new has no fallible steps given a temp dir")
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime builds")
        .block_on(f)
}

#[test]
fn version_async_returns_advertised_without_fetch() {
    let project = make_fixture();
    let version = block_on(project.version_async()).unwrap();
    assert_eq!(version.as_deref(), Some("1.2.3"));
    assert!(
        !project.archive.is_downloaded_and_verified(),
        "version_async must not trigger a download"
    );
}

#[test]
fn usage_async_returns_advertised_without_fetch() {
    let project = make_fixture();
    let usage = block_on(project.usage_async())
        .unwrap()
        .expect("advertised usage is Some");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].resource, format!("{PKG_SYSAND_PREFIX}acme/widget"));
    assert!(!project.archive.is_downloaded_and_verified());
}

#[test]
fn checksum_canonical_hex_async_returns_advertised_before_download() {
    let project = make_fixture();
    let digest = block_on(project.checksum_canonical_hex_async()).unwrap();
    assert_eq!(
        digest.as_deref(),
        Some("1111111111111111111111111111111111111111111111111111111111111111"),
        "pre-download, checksum_canonical_hex_async must return the advertised digest verbatim \
         (no archive download, no kpar-side computation)"
    );
    assert!(
        !project.archive.is_downloaded_and_verified(),
        "checksum_canonical_hex_async must not trigger a download before the archive is present"
    );
}
