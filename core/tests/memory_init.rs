// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use sysand_core::{
    commands::init::{GivenProject, InitError, do_init_toml},
    init::do_init_memory,
    project::{
        ManifestWriteError, ProjectDefinition, ProjectFormat, ProjectRead as _,
        memory::{InMemoryError, InMemoryProject},
    },
    utils::format_err,
};

/// The metamodel every project here declares: `init` takes one, it
/// does not choose one.
const SYSML: &str = "https://www.omg.org/spec/SysML/20250201";

/// `sysand init` creates a `model.toml`, not the two json documents;
/// those are what a build produces and what a json project has.
#[test]
fn init_basic() -> Result<(), Box<dyn std::error::Error>> {
    let memory_storage = do_init_memory(
        "init-basic",
        "echo",
        "1.2.3",
        Some("Apache-2.0".to_owned()),
        SYSML.to_owned(),
    )?;

    assert_eq!(memory_storage.format()?, ProjectFormat::Toml);
    assert_eq!(
        memory_storage.model_toml.as_deref(),
        Some(
            r#"manifest-version = "1.0"

[project]
publisher = "echo"
name = "init-basic"
version = "1.2.3"
sources = []
license = "Apache-2.0"
metamodel = "https://www.omg.org/spec/SysML/20250201"
"#
        )
    );
    assert!(memory_storage.info.is_none());
    assert!(memory_storage.meta.is_none());

    let Some(ProjectDefinition::Toml(manifest)) = memory_storage.get_project().expect("a manifest")
    else {
        panic!("a `model.toml` project");
    };
    assert_eq!(manifest.name.as_str(), "init-basic");
    assert_eq!(manifest.publisher.as_str(), "echo");
    assert_eq!(
        manifest.sources().expect("an OMG project").as_slice(),
        Vec::<typed_path::Utf8UnixPathBuf>::new()
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
        SYSML.to_owned(),
    )?;

    let original = memory_storage.model_toml.clone();

    let second_result = do_init_toml(
        given(
            "init-fail-on-double-init",
            "acme",
            Some("Apache-2.0 OR MIT"),
            SYSML,
        )?
        .into(),
        &mut memory_storage,
    );

    // The refusal comes back through the manifest writer, which is
    // what `init` stores through.
    assert_matches!(
        second_result,
        Err(InitError::TomlWrite(ManifestWriteError::Project(
            InMemoryError::AlreadyExists(_)
        )))
    );

    assert_eq!(memory_storage.model_toml, original);

    Ok(())
}

/// `do_init_memory` should reject an invalid project name, reporting the
/// offending input
#[test]
fn init_rejects_invalid_name() {
    let result = do_init_memory("a/b", "echo", "1.2.3", None, SYSML.to_owned());

    let Err(err @ InitError::ProjectField(..)) = result else {
        panic!("expected `ProjectField`, got {result:?}");
    };
    assert!(format_err(&err).contains("`a/b`"), "{err}");
}

/// `do_init_memory` should reject an invalid project publisher, reporting
/// the offending input
#[test]
fn init_rejects_invalid_publisher() {
    let result = do_init_memory("name", "acme:labs", "1.2.3", None, SYSML.to_owned());

    let Err(err @ InitError::ProjectField(..)) = result else {
        panic!("expected `ProjectField`, got {result:?}");
    };
    assert!(format_err(&err).contains("`acme:labs`"), "{err}");
}

/// A name a `model.toml` refuses should be reported and leave the storage
/// untouched
#[test]
fn init_invalid_name_leaves_storage_empty() -> Result<(), Box<dyn std::error::Error>> {
    let mut storage = InMemoryProject::default();

    let result = do_init_toml(
        given("my_widgets", "echo", None, SYSML)?.into(),
        &mut storage,
    );

    let Err(err @ InitError::Identifier(..)) = result else {
        panic!("expected `Identifier`, got {result:?}");
    };
    assert!(format_err(&err).contains("`my_widgets`"), "{err}");
    assert!(storage.model_toml.is_none());
    assert!(storage.info.is_none());
    assert!(storage.meta.is_none());
    Ok(())
}

/// `do_init_toml` should write the metamodel given to `model.toml`
#[test]
fn init_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let mut storage = InMemoryProject::default();

    do_init_toml(given("name", "echo", None, SYSML)?.into(), &mut storage)?;

    let manifest = storage.model_toml.expect("a model.toml");
    assert!(
        manifest.contains("metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n"),
        "{manifest}"
    );
    Ok(())
}

/// A metamodel Sysand does not recognize should be refused, reporting it
#[test]
fn init_rejects_invalid_metamodel() {
    let result = given("name", "echo", None, "not an iri");

    let Err(err @ InitError::Metamodel(..)) = result else {
        panic!("expected `Metamodel`, got {result:?}");
    };
    assert!(
        err.to_string()
            .starts_with("`not an iri` is not a metamodel Sysand recognizes"),
        "{err}"
    );
}

/// A project of version `1.2.3`, from the other keys as written.
fn given(
    name: &str,
    publisher: &str,
    license: Option<&str>,
    metamodel: &str,
) -> Result<GivenProject, InitError<InMemoryError>> {
    GivenProject::parse(
        name.to_owned(),
        publisher.to_owned(),
        "1.2.3".to_owned(),
        license.map(str::to_owned),
        metamodel.to_owned(),
    )
}
