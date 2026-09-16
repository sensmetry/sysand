// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use sysand_core::{
    commands::init::do_init,
    init::do_init_memory,
    manifest::inherit::MetaProperty,
    project::{ManifestWriteError, ProjectFormat, ProjectRead as _},
};

/// The metamodel every project here declares: `init` takes one, it
/// does not choose one.
const SYSML: &str = "https://www.omg.org/spec/SysML/20250201";

/// `sysand init` creates a `model.toml`, not the two json documents;
/// those are what a build produces and what a legacy project has.
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let memory_storage = do_init_memory(
        "init-basic",
        "echo",
        "1.2.3",
        Some("Apache-2.0".to_owned()),
        Some(MetaProperty::Provided(SYSML.to_owned())),
    )?;

    assert_eq!(memory_storage.format()?, ProjectFormat::Toml);
    assert_eq!(
        memory_storage.model_toml.as_deref(),
        Some(
            "manifest-version = \"1.0.0\"\n\n\
             [project]\n\
             publisher = \"echo\"\n\
             name = \"init-basic\"\n\
             version = \"1.2.3\"\n\
             sources = []\n\
             license = \"Apache-2.0\"\n\
             metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"
        )
    );
    assert!(memory_storage.info.is_none());
    assert!(memory_storage.meta.is_none());

    // What a json-shaped caller sees is synthesized from the manifest.
    let manifest = memory_storage.manifest().expect("a manifest");
    assert_eq!(manifest.name.as_str(), "init-basic");
    assert_eq!(manifest.publisher.as_str(), "echo");
    assert_eq!(
        manifest.sources.as_slice(),
        [] as [typed_path::Utf8UnixPathBuf; 0]
    );

    Ok(())
}

/// `sysand init` should fail (loudly) in case there is already
/// a project present (in the current working directory). The current
/// project should remain unaffected by the second `sysand init` execution.
#[test]
fn init_fail_on_double_init() -> Result<(), Box<dyn std::error::Error>> {
    let mut memory_storage = do_init_memory(
        "init-fail-on-double-init",
        "acme",
        "1.2.3",
        Some("Apache-2.0 OR MIT".to_owned()),
        Some(MetaProperty::Provided(SYSML.to_owned())),
    )?;

    let original = memory_storage.model_toml.clone();

    let second_result = do_init(
        "init-fail-on-double-init".to_owned(),
        "acme".into(),
        "1.2.3".to_owned(),
        Some("Apache-2.0 OR MIT".to_owned()),
        Some(MetaProperty::Provided(SYSML.to_owned())),
        &mut memory_storage,
    );

    // The refusal comes back through the manifest writer, which is
    // what `init` stores through.
    assert!(matches!(
        second_result,
        Err(sysand_core::commands::init::InitError::Write(
            ManifestWriteError::Project(
                sysand_core::project::memory::InMemoryError::AlreadyExists(_)
            )
        ))
    ));

    assert_eq!(memory_storage.model_toml, original);

    Ok(())
}

/// `do_init` (and so `do_init_memory`) should reject an invalid project
/// name, reporting the offending input
#[test]
fn init_rejects_invalid_name() {
    let result = do_init_memory(
        "a/b",
        "echo",
        "1.2.3",
        None,
        Some(MetaProperty::Provided(SYSML.to_owned())),
    );

    let Err(err @ sysand_core::commands::init::InitError::Identifier(..)) = result else {
        panic!("expected `Identifier`, got {result:?}");
    };
    assert!(err.to_string().contains("`a/b`"), "{err}");
}

/// `do_init` (and so `do_init_memory`) should reject an invalid project
/// publisher, reporting the offending input
#[test]
fn init_rejects_invalid_publisher() {
    let result = do_init_memory(
        "name",
        "acme:labs",
        "1.2.3",
        None,
        Some(MetaProperty::Provided(SYSML.to_owned())),
    );

    let Err(err @ sysand_core::commands::init::InitError::Identifier(..)) = result else {
        panic!("expected `Identifier`, got {result:?}");
    };
    assert!(err.to_string().contains("`acme:labs`"), "{err}");
}

/// An invalid name or publisher should leave the storage untouched
#[test]
fn init_invalid_name_leaves_storage_empty() {
    let mut storage = sysand_core::project::memory::InMemoryProject::default();

    let result = do_init(
        String::new(),
        "echo".to_owned(),
        "1.2.3".to_owned(),
        None,
        None,
        &mut storage,
    );

    assert_matches!(
        result,
        Err(sysand_core::commands::init::InitError::Identifier(..))
    );
    assert!(storage.model_toml.is_none());
    assert!(storage.info.is_none());
    assert!(storage.meta.is_none());
}

/// `do_init` should write the metamodel given to `model.toml`
#[test]
fn init_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let mut storage = sysand_core::project::memory::InMemoryProject::default();

    do_init(
        "name".to_owned(),
        "echo".to_owned(),
        "1.2.3".to_owned(),
        None,
        Some(MetaProperty::Provided(SYSML.to_owned())),
        &mut storage,
    )?;

    let manifest = storage.model_toml.expect("a model.toml");
    assert!(
        manifest.contains("metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"),
        "{manifest}"
    );
    Ok(())
}

/// `do_init` should reject a metamodel Sysand does not recognize,
/// reporting it, and leave the storage untouched
#[test]
fn init_rejects_invalid_metamodel() {
    let mut storage = sysand_core::project::memory::InMemoryProject::default();

    let result = do_init(
        "name".to_owned(),
        "echo".to_owned(),
        "1.2.3".to_owned(),
        None,
        Some(MetaProperty::Provided("not an iri".to_owned())),
        &mut storage,
    );

    let Err(err @ sysand_core::commands::init::InitError::Metamodel(..)) = result else {
        panic!("expected `Metamodel`, got {result:?}");
    };
    assert!(
        err.to_string()
            .starts_with("`not an iri` is not a metamodel Sysand recognizes"),
        "{err}"
    );
    assert!(storage.model_toml.is_none());
    assert!(storage.info.is_none());
    assert!(storage.meta.is_none());
}
