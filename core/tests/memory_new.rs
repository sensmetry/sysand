// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use semver::Version;
use sysand_core::{commands::new::do_new, model::InterchangeProjectInfo, new::do_new_memory};

/// `sysand init` should create valid, minimal, .project.json
/// and .meta.json files in the current working directory. (Non-interactive use)
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let memory_storage = do_new_memory("init_basic".to_string(), "1.2.3".to_string())?;

    assert_eq!(
        memory_storage.info.unwrap(),
        InterchangeProjectInfo {
            name: "init_basic".to_string(),
            description: None,
            version: Version::parse("1.2.3").unwrap(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }
        .into()
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
    let mut memory_storage =
        do_new_memory("init_fail_on_double_init".to_string(), "1.2.3".to_string())?;

    let original_info = memory_storage.info.clone();
    let original_meta = memory_storage.meta.clone();

    let second_result = do_new(
        "init_fail_on_double_init".to_string(),
        "1.2.3".to_string(),
        &mut memory_storage,
    );

    assert!(matches!(
        second_result,
        Err(sysand_core::commands::new::NewError::Project(
            sysand_core::project::memory::InMemoryError::AlreadyExists(_)
        ))
    ));

    assert_eq!(memory_storage.info, original_info,);

    assert_eq!(memory_storage.meta, original_meta,);

    Ok(())
}
