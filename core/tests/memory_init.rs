// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use sysand_core::{
    commands::init::do_init_parse, init::do_init_memory, model::InterchangeProjectInfoRaw,
};

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the current working directory. (Non-interactive use)
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let memory_storage = do_init_memory("init_basic", "e", "1.2.3", Some("Apache-2.0".to_owned()))?;

    assert_eq!(
        memory_storage.info.unwrap(),
        InterchangeProjectInfoRaw {
            name: "init_basic".to_owned(),
            publisher: Some("e".to_owned()),
            description: None,
            version: "1.2.3".to_owned(),
            license: Some("Apache-2.0".to_owned()),
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }
    );

    assert!(memory_storage.meta.as_ref().unwrap().index.is_empty());
    assert!(memory_storage.meta.as_ref().unwrap().metamodel.is_none());

    assert!(
        memory_storage
            .meta
            .as_ref()
            .unwrap()
            .includes_derived
            .is_none()
    );
    assert!(
        memory_storage
            .meta
            .as_ref()
            .unwrap()
            .includes_implied
            .is_none()
    );
    assert!(memory_storage.meta.as_ref().unwrap().checksum.is_none());

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the current working directory). The current
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init() -> Result<(), Box<dyn std::error::Error>> {
    let mut memory_storage = do_init_memory(
        "init_fail_on_double_init",
        "a",
        "1.2.3",
        Some("Apache-2.0 OR MIT".to_owned()),
    )?;

    let original_info = memory_storage.info.clone();
    let original_meta = memory_storage.meta.clone();

    let second_result = do_init_parse(
        "init_fail_on_double_init".to_owned(),
        "a".into(),
        "1.2.3".to_owned(),
        Some("Apache-2.0 OR MIT".to_owned()),
        &mut memory_storage,
    );

    assert_matches!(
        second_result,
        Err(sysand_core::commands::init::InitError::Project(
            sysand_core::project::memory::InMemoryError::AlreadyExists(_)
        ))
    );

    assert_eq!(memory_storage.info, original_info);
    assert_eq!(memory_storage.meta, original_meta);

    Ok(())
}

/// `do_init_parse` (and so `do_init_memory`) should reject an invalid
/// project name, reporting the offending input
#[test]
fn init_rejects_invalid_name() {
    let result = do_init_memory("a/b", "e", "1.2.3", None);

    let Err(err @ sysand_core::commands::init::InitError::NameParse(..)) = result else {
        panic!("expected `NameParse`, got {result:?}");
    };
    assert_eq!(
        err.to_string(),
        "invalid project name `a/b`: name cannot contain `/`"
    );
}

/// `do_init_parse` (and so `do_init_memory`) should reject an invalid
/// project publisher, reporting the offending input
#[test]
fn init_rejects_invalid_publisher() {
    let result = do_init_memory("n", "acme:labs", "1.2.3", None);

    let Err(err @ sysand_core::commands::init::InitError::PublisherParse(..)) = result else {
        panic!("expected `PublisherParse`, got {result:?}");
    };
    assert_eq!(
        err.to_string(),
        "invalid project publisher `acme:labs`: publisher cannot contain `:`"
    );
}

/// An invalid name or publisher should leave the storage untouched
#[test]
fn init_invalid_name_leaves_storage_empty() {
    let mut storage = sysand_core::project::memory::InMemoryProject::default();

    let result = do_init_parse(
        String::new(),
        "e".to_owned(),
        "1.2.3".to_owned(),
        None,
        &mut storage,
    );

    assert_matches!(
        result,
        Err(sysand_core::commands::init::InitError::NameParse(..))
    );
    assert!(storage.info.is_none());
    assert!(storage.meta.is_none());
}
