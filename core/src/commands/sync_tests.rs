use std::convert::Infallible;

use chrono::DateTime;
use indexmap::IndexMap;
use semver::Version;

use crate::{
    env::{
        ReadEnvironment, WriteEnvironment, memory::MemoryStorageEnvironment, utils::clone_project,
    },
    model::{InterchangeProjectInfo, InterchangeProjectMetadata},
    project::{ProjectMut, ProjectRead, memory::InMemoryProject},
    sync::{SyncError, is_installed, try_install},
};

fn storage_example() -> InMemoryProject {
    let mut storage = InMemoryProject::new();

    storage
        .put_project(
            &InterchangeProjectInfo {
                name: "install_test".to_string(),
                publisher: None,
                description: None,
                version: Version::new(1, 2, 3),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: vec![],
            }
            .into(),
            &InterchangeProjectMetadata {
                index: IndexMap::new(),
                created: DateTime::from_timestamp(1, 2).unwrap(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: None,
            }
            .into(),
            true,
        )
        .unwrap();

    storage
}

#[test]
fn test_is_not_installed() {
    let uri = "urn:kpar:install_test";
    let checksum = "00";
    let env = MemoryStorageEnvironment::new();

    assert!(
        !is_installed::<MemoryStorageEnvironment<InMemoryProject>, Infallible, Infallible, _, _>(
            uri, checksum, &env
        )
        .unwrap()
    );
}

#[test]
fn test_is_installed() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let checksum = storage.checksum_non_canonical_hex().unwrap().unwrap();
    let mut env = MemoryStorageEnvironment::new();
    env.put_project(uri, "1,2,3", |p| {
        clone_project(&storage, p, true).map(|_| ())
    })
    .unwrap();

    assert!(
        is_installed::<MemoryStorageEnvironment<InMemoryProject>, Infallible, Infallible, _, _>(
            uri, &checksum, &env
        )
        .unwrap()
    );

    assert!(
        !is_installed::<MemoryStorageEnvironment<InMemoryProject>, Infallible, Infallible, _, _>(
            uri, "00", &env
        )
        .unwrap()
    );

    assert!(
        !is_installed::<MemoryStorageEnvironment<InMemoryProject>, Infallible, Infallible, _, _>(
            "not_uri", &checksum, &env
        )
        .unwrap()
    );
}

#[test]
fn test_try_install() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let checksum = storage.checksum_non_canonical_hex().unwrap().unwrap();
    let mut env = MemoryStorageEnvironment::new();

    try_install::<
        MemoryStorageEnvironment<InMemoryProject>,
        InMemoryProject,
        Infallible,
        Infallible,
        _,
        _,
    >(uri, &checksum, storage, &mut env)
    .unwrap();

    let uris = env.uris().unwrap();

    assert_eq!(uris.len(), 1);
    assert_eq!(uris.first().unwrap().as_ref().unwrap(), uri);

    let versions = env.versions(uri).unwrap();

    assert_eq!(versions.len(), 1);
    assert_eq!(versions.first().unwrap().as_ref().unwrap(), "1.2.3");
}

#[test]
fn test_try_install_bad_checksum() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let checksum = "00";
    let mut env = MemoryStorageEnvironment::new();

    let SyncError::BadChecksum(msg) = try_install::<
        MemoryStorageEnvironment<InMemoryProject>,
        InMemoryProject,
        Infallible,
        Infallible,
        _,
        _,
    >(&uri, &checksum, storage, &mut env)
    .unwrap_err() else {
        panic!()
    };

    assert_eq!(msg, uri);

    let uris = env.uris().unwrap();

    assert_eq!(uris.len(), 0);
}
