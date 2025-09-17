// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

use chrono::DateTime;
use indexmap::IndexMap;
use semver::Version;
use sysand_core::{
    commands::env::do_env_memory,
    env::{
        ReadEnvironment, WriteEnvironment,
        utils::{CloneError, clone_project},
    },
    info::do_info,
    model::{InterchangeProjectInfo, InterchangeProjectMetadata},
    project::{
        ProjectMut, ProjectRead,
        memory::{InMemoryError, InMemoryProject},
    },
    resolve::memory::{AcceptAll, MemoryResolver},
};
use typed_path::Utf8UnixPath;

#[test]
fn env_basic() -> Result<(), Box<dyn std::error::Error>> {
    let memory_environment = do_env_memory()?;

    assert!(memory_environment.projects.is_empty());

    Ok(())
}

#[test]
fn env_manual_install() -> Result<(), Box<dyn std::error::Error>> {
    let mut memory_environment = do_env_memory()?;

    let info = InterchangeProjectInfo {
        name: "env_manual_install".to_string(),
        description: None,
        version: Version::new(1, 2, 3),
        license: None,
        maintainer: vec![],
        website: None,
        topic: vec![],
        usage: vec![],
    }
    .into();

    let mut index = IndexMap::new();
    index.insert(
        "SomePackage".to_string(),
        Utf8UnixPath::new("SomePackage.sysml").to_path_buf(),
    );

    let meta = InterchangeProjectMetadata {
        index,
        created: DateTime::from_timestamp(1, 2).unwrap(),
        metamodel: None,
        includes_derived: None,
        includes_implied: None,
        checksum: None,
    }
    .into();

    let mut source_project = InMemoryProject::default();

    source_project.put_project(&info, &meta, true)?;

    let source_path = Utf8UnixPath::new("SomePackage.sysml");
    let source_code = "package SomePackage;";

    source_project.write_source(source_path, &mut Cursor::new(source_code), true)?;

    memory_environment.put_project("urn::sysand_test::1", "1.2.3", |p| {
        clone_project(&source_project, p, true)?;

        Ok::<(), CloneError<InMemoryError, InMemoryError>>(())
    })?;

    let target_project = memory_environment.get_project("urn::sysand_test::1", "1.2.3")?;

    assert_eq!(target_project.info, Some(info.clone()));
    assert_eq!(target_project.meta, Some(meta.clone()));

    let mut read_source_code = "".to_string();

    target_project
        .read_source(source_path)?
        .read_to_string(&mut read_source_code)?;

    assert_eq!(read_source_code, source_code);

    let resolver = MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::from([(
            fluent_uri::Iri::parse("urn::sysand_test::1".to_string())?,
            source_project.clone(),
        )]),
    };

    let resolved_projects = do_info("urn::sysand_test::1", &resolver)?;

    assert_eq!(resolved_projects.len(), 1);
    assert_eq!(resolved_projects[0], (info, meta));

    Ok(())
}
