// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use crate::{
    commands::lock::{LockError, do_lock_extend, do_lock_projects},
    lock::{Lock, Project, Source},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::memory::InMemoryProject,
    resolve::null::NullResolver,
};

#[test]
fn lock_export_conflict() {
    let exports = vec!["sym1".into(), "sym2".into(), "sym3".into()];

    let lock = Lock {
        lock_version: String::new(),
        projects: vec![
            Project {
                name: "test1".into(),
                publisher: None,
                version: String::new(),
                exports: exports.clone(),
                identifiers: vec!["test1".into()],
                sources: vec![],
                usages: vec![],
            },
            Project {
                name: "test2".into(),
                publisher: None,
                version: String::new(),
                exports,
                identifiers: vec!["test2".into()],
                sources: vec![],
                usages: vec![],
            },
        ],
    };
    let res = do_lock_extend(
        lock,
        [],
        NullResolver {},
        &HashMap::new(),
        &Default::default(),
    );

    assert!(matches!(res, Err(LockError::NameCollision(_))));
}

#[test]
fn lock_preserves_project_publisher() {
    let mut project = InMemoryProject::from_info_meta(
        InterchangeProjectInfoRaw {
            name: "published_project".into(),
            publisher: Some("Acme Labs".into()),
            version: "1.2.3".into(),
            description: None,
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        },
        InterchangeProjectMetadataRaw {
            index: Default::default(),
            created: "2026-01-01T00:00:00Z".into(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        },
    );
    project.nominal_sources = vec![Source::Editable {
        editable: ".".into(),
    }];

    let lock = do_lock_projects(
        [(None, &project)],
        NullResolver {},
        &HashMap::new(),
        &Default::default(),
    )
    .unwrap()
    .lock;

    assert_eq!(lock.projects[0].publisher.as_deref(), Some("Acme Labs"));
}
