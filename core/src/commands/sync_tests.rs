// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::convert::Infallible;

use chrono::DateTime;
use indexmap::IndexMap;
use semver::Version;

use crate::{
    env::{
        ProjectChecksumResult, ReadEnvironment, WriteEnvironment, memory::MemoryStorageEnvironment,
        utils::clone_project,
    },
    model::{InterchangeProjectInfo, InterchangeProjectMetadata},
    project::{ProjectChecksum, ProjectMut, ProjectRead, memory::InMemoryProject},
    sync::{SyncError, try_install},
};

fn new_env() -> MemoryStorageEnvironment<InMemoryProject> {
    MemoryStorageEnvironment::<InMemoryProject>::new()
}

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
fn not_installed_project_not_found() {
    let uri = "urn:kpar:install_test";
    let env = new_env();

    assert!(!env.has(uri).unwrap());
}

#[test]
fn installed_projects_are_found() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let version = "1.2.3";
    let checksum = storage.checksum_canonical_variant().unwrap();
    let mut env = new_env();
    env.put_project(uri, version, Some(checksum.clone()), |p| {
        clone_project(&storage, p, true).map(|_| ())
    })
    .unwrap();

    assert_eq!(
        env.has_version_verified(uri, version, &checksum).unwrap(),
        ProjectChecksumResult::Match
    );

    assert_eq!(
        env.has_version_verified(
            uri,
            version,
            &crate::project::ProjectChecksum::Project(String::from("00"))
        )
        .unwrap(),
        ProjectChecksumResult::Mismatch
    );

    assert_eq!(
        env.has_version_verified("not_uri", version, &checksum)
            .unwrap(),
        ProjectChecksumResult::VersionNotFound
    );
}

#[test]
fn try_install_installs_project() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let checksum = storage.checksum_canonical_variant().unwrap();
    let mut env = new_env();

    try_install::<_, InMemoryProject, Infallible, Infallible, _>(
        uri, "1.2.3", &checksum, storage, &mut env,
    )
    .unwrap();

    let uris = env.uris().unwrap();

    assert_eq!(uris.len(), 1);
    assert_eq!(uris.first().unwrap().as_ref().unwrap(), uri);

    let versions = env.versions(uri).unwrap();

    assert_eq!(versions.len(), 1);
    assert_eq!(versions.first().unwrap().as_ref().unwrap(), "1.2.3");
}

#[test]
fn try_install_fails_to_install_wrong_checksum() {
    let storage = storage_example();

    let uri = "urn:kpar:install_test";
    let checksum = ProjectChecksum::Project("00".to_owned());
    let mut env = new_env();

    let SyncError::BadChecksum {
        id: iri,
        expected,
        actual,
    } = try_install::<_, _, Infallible, Infallible, _>(
        &uri, "1.2.3", &checksum, &storage, &mut env,
    )
    .unwrap_err()
    else {
        panic!()
    };

    assert_eq!(iri, uri);
    assert_eq!(expected, checksum);
    assert_eq!(
        actual,
        ProjectChecksum::Project(
            "9612357c41e64a174582b813ca2f1695fd11baf7166624ea2bdf76b406ee2d71".to_owned()
        )
    );

    let uris = env.uris().unwrap();

    assert_eq!(uris.len(), 0);
}

#[test]
fn has_version_verified_different_checksum_kinds() {
    let storage = storage_example();
    let uri = "urn:kpar:install_test";
    let version = "1.2.3";
    let project_checksum = storage.checksum_canonical_variant().unwrap();
    // InMemoryProject always produces ProjectChecksum::Project; flip to Kpar to trigger mismatch
    let kpar_checksum = match &project_checksum {
        ProjectChecksum::Project(c) => ProjectChecksum::Kpar(c.clone()),
        ProjectChecksum::Kpar(c) => ProjectChecksum::Project(c.clone()),
    };
    let mut env = new_env();
    env.put_project(uri, version, Some(project_checksum), |p| {
        clone_project(&storage, p, true).map(|_| ())
    })
    .unwrap();

    assert_eq!(
        env.has_version_verified(uri, version, &kpar_checksum)
            .unwrap(),
        ProjectChecksumResult::DifferentChecksumKinds
    );
}

#[test]
fn has_version_verified_version_not_found_for_known_uri() {
    let storage = storage_example();
    let uri = "urn:kpar:install_test";
    let version = "1.2.3";
    let checksum = storage.checksum_canonical_variant().unwrap();
    let mut env = new_env();
    env.put_project(uri, version, Some(checksum.clone()), |p| {
        clone_project(&storage, p, true).map(|_| ())
    })
    .unwrap();

    assert_eq!(
        env.has_version_verified(uri, "9.9.9", &checksum).unwrap(),
        ProjectChecksumResult::VersionNotFound
    );
}
