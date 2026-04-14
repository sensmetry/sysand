// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use crate::{
    env::{
        ReadEnvironment, WriteEnvironment,
        memory::MemoryStorageEnvironment,
        utils::{CloneError, clone_project},
    },
    init::do_init_memory,
    project::memory::{InMemoryError, InMemoryProject},
};

#[test]
fn write_environment() {
    let uri1 = "urn:kpar:first".to_string();
    let uri2 = "urn:kpar:second".to_string();
    let version = "0.0.1".to_string();
    let project1 = do_init_memory("First", Some("a"), &version, None).unwrap();
    let project2 = do_init_memory("Second", None::<&str>, &version, None).unwrap();
    let mut env = MemoryStorageEnvironment::<InMemoryProject>::new();

    env.put_project(&uri1, &version, |p| {
        clone_project(&project1, p, true)?;

        Ok::<(), CloneError<InMemoryError, InMemoryError>>(())
    })
    .unwrap();

    assert_eq!(env.projects.len(), 1);
    assert_eq!(
        &project1,
        env.projects.get(&uri1).unwrap().get(&version).unwrap()
    );

    env.put_project(&uri2, &version, |p| {
        clone_project(&project2, p, true)?;

        Ok::<(), CloneError<InMemoryError, InMemoryError>>(())
    })
    .unwrap();

    assert_eq!(env.projects.len(), 2);
    assert_eq!(
        &project2,
        env.projects.get(&uri2).unwrap().get(&version).unwrap()
    );

    env.del_project_version(&uri1, version).unwrap();

    assert_eq!(env.projects.len(), 1);
    assert!(!env.projects.contains_key(&uri1));

    env.del_uri(&uri2).unwrap();

    assert!(env.projects.is_empty());
    assert!(!env.projects.contains_key(&uri2));
}

#[test]
fn read_environment() {
    let iri = "urn:kpar:first".to_string();
    let version = "0.0.1".to_string();
    let project = do_init_memory("First", Some("a"), &version, None).unwrap();
    let env = MemoryStorageEnvironment {
        projects: HashMap::from([(
            iri.clone(),
            HashMap::from([(version.clone(), project.clone())]),
        )]),
    };

    let uris = env.uris().unwrap();
    assert_eq!(
        vec![&iri],
        uris.iter()
            .map(|uri| uri.as_ref().unwrap())
            .collect::<Vec<_>>()
    );

    let versions = env.versions(&iri).unwrap();
    assert_eq!(
        vec![&version],
        versions
            .iter()
            .map(|version| version.as_ref().unwrap())
            .collect::<Vec<_>>()
    );

    let get_project = env.get_project(iri, version).unwrap();
    assert_eq!(project, get_project);
}

#[test]
fn from() {
    let version1 = "0.0.1".to_string();
    let version2 = "0.1.0".to_string();
    let version3 = "0.0.1".to_string();
    let project1 = do_init_memory("First 0.0.1", Some("a"), &version1, None).unwrap();
    let project2 = do_init_memory("First 0.1.0", None::<&str>, &version2, None).unwrap();
    let project3 = do_init_memory("Second", Some("a"), &version3, None).unwrap();
    let env = MemoryStorageEnvironment::<InMemoryProject>::from([
        ("urn:kpar:first".into(), version1.clone(), project1.clone()),
        ("urn:kpar:first".into(), version2.clone(), project2.clone()),
        ("urn:kpar:second".into(), version3.clone(), project3.clone()),
    ]);

    assert_eq!(
        project1,
        env.get_project("urn:kpar:first", version1).unwrap()
    );
    assert_eq!(
        project2,
        env.get_project("urn:kpar:first", version2).unwrap()
    );
    assert_eq!(
        project3,
        env.get_project("urn:kpar:second", version3).unwrap()
    );
    assert_eq!(env.projects.len(), 2);
    assert_eq!(env.projects.get("urn:kpar:first").unwrap().len(), 2);
}

#[test]
fn try_from() {
    let project1 = do_init_memory("First 0.0.1", Some("a"), "0.0.1", None).unwrap();
    let project2 = do_init_memory("First 0.1.0", Some("a"), "0.1.0", None).unwrap();
    let project3 = do_init_memory("Second", Some("a"), "0.0.1", None).unwrap();
    let env = MemoryStorageEnvironment::<InMemoryProject>::try_from([
        ("urn:kpar:first".into(), project1.clone()),
        ("urn:kpar:first".into(), project2.clone()),
        ("urn:kpar:second".into(), project3.clone()),
    ])
    .unwrap();

    assert_eq!(
        project1,
        env.get_project("urn:kpar:first", "0.0.1").unwrap()
    );
    assert_eq!(
        project2,
        env.get_project("urn:kpar:first", "0.1.0").unwrap()
    );
    assert_eq!(
        project3,
        env.get_project("urn:kpar:second", "0.0.1").unwrap()
    );
    assert_eq!(env.projects.len(), 2);
    assert_eq!(env.projects.get("urn:kpar:first").unwrap().len(), 2);
}
