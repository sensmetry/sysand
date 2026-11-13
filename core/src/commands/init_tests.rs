// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use crate::commands::init::{InitError, do_init_memory};
use crate::manifest::common::InheritFrom;
use crate::manifest::inherit::{Inherited, MetaProperty};

const OMG_SYSML: &str = "https://www.omg.org/spec/SysML/20250201";

fn init(metamodel: Option<MetaProperty<String>>) -> String {
    do_init_memory("widgets", "acme", "1.2.3", None, metamodel)
        .expect("the project is created")
        .model_toml
        .expect("a manifest")
}

/// A project declares a metamodel, and `init` is given it.
#[test]
fn a_metamodel_is_written_when_given() {
    let written = init(Some(MetaProperty::Provided(OMG_SYSML.to_owned())));
    assert!(
        written.contains(&format!("\nmetamodel = \"{OMG_SYSML}\"\n")),
        "unexpected manifest: {written}"
    );
}

/// `metamodel.workspace = true` satisfies the metamodel requirement in
/// `[project]`. This is what `init` writes inside a workspace whose default
/// group declares a metamodel.
#[test]
fn a_marker_is_written_when_asked_for() {
    let written = init(Some(MetaProperty::Inherited(Inherited {
        from: InheritFrom::DefaultGroup,
        at: None,
    })));
    assert!(
        written.contains("\nmetamodel.workspace = true\n"),
        "unexpected manifest: {written}"
    );
}

/// A metamodel outside the recognized values is refused.
#[test]
fn an_unrecognized_metamodel_is_refused() {
    let error = do_init_memory(
        "widgets",
        "acme",
        "1.2.3",
        None,
        Some(MetaProperty::Provided(
            "https://example.com/spec".to_owned(),
        )),
    )
    .expect_err("an unrecognized metamodel is refused");
    assert_matches!(&error, InitError::Metamodel(value) if &**value == "https://example.com/spec", "unexpected error: {error}");
    assert!(
        error
            .to_string()
            .contains("https://www.sysand.com/spec/SysML/<date>"),
        "the message lists the four values: {error}"
    );
}

/// `init` takes the metamodel as an option: given none, it writes no
/// `metamodel` key.
#[test]
fn no_metamodel_is_written_when_none_is_given() {
    assert!(!init(None).contains("metamodel"));
}

/// `init` with a Syside metamodel writes the Syside layout and creates the root file, through
/// the same storage the manifest goes to.
#[test]
fn init_with_a_syside_metamodel_writes_the_layout_and_the_root_file() {
    let project = super::do_init_memory(
        "My Widgets",
        "acme",
        "1.0.0",
        None,
        Some(crate::manifest::inherit::MetaProperty::Provided(
            "https://www.sysand.com/spec/KerML/20250201".to_owned(),
        )),
    )
    .expect("the project is created");
    let manifest = crate::project::ProjectRead::manifest_document(&project)
        .expect("a manifest")
        .to_string();
    assert!(!manifest.contains("sources"), "{manifest}");
    assert!(
        manifest.contains("root = \"my-widgets.kerml\"\n"),
        "{manifest}"
    );
    assert_eq!(
        project
            .files
            .get(&typed_path::Utf8UnixPathBuf::from("my-widgets.kerml")),
        Some(&"package 'my-widgets';\n".to_owned()),
        "a name with a separator is a quoted package name"
    );
}

/// An existing root file is left alone.
#[test]
fn init_does_not_overwrite_an_existing_root_file() {
    let mut storage = crate::project::memory::InMemoryProject::default();
    storage
        .files
        .insert("widgets.sysml".into(), "package Widgets { }\n".to_owned());
    super::do_init_toml(
        "widgets".to_owned(),
        "acme".to_owned(),
        "1.0.0".to_owned(),
        None,
        Some(crate::manifest::inherit::MetaProperty::Provided(
            "https://www.sysand.com/spec/SysML/20250201".to_owned(),
        )),
        &mut storage,
    )
    .expect("the project is created");
    assert_eq!(
        storage
            .files
            .get(&typed_path::Utf8UnixPathBuf::from("widgets.sysml")),
        Some(&"package Widgets { }\n".to_owned())
    );
}

/// A marker resolves to whatever group the caller names; the layout
/// follows the group's metamodel when the caller says what it is, and is
/// the OMG one otherwise.
#[test]
fn init_with_a_marker_takes_the_layout_from_the_inherited_kind() {
    let marker = || {
        Some(crate::manifest::inherit::MetaProperty::Inherited(
            crate::manifest::inherit::Inherited {
                from: crate::manifest::common::InheritFrom::DefaultGroup,
                at: None,
            },
        ))
    };
    let mut syside = crate::project::memory::InMemoryProject::default();
    super::do_init_toml_ext(
        "widgets".to_owned(),
        "acme".to_owned(),
        "1.0.0".to_owned(),
        None,
        marker(),
        crate::model::KnownMetamodel::recognize("https://www.sysand.com/spec/SysML/20250201"),
        &mut syside,
    )
    .expect("created");
    let manifest = crate::project::ProjectRead::manifest_document(&syside)
        .expect("a manifest")
        .to_string();
    assert!(
        manifest.contains("metamodel.workspace = true"),
        "{manifest}"
    );
    assert!(manifest.contains("root = \"widgets.sysml\""), "{manifest}");

    let mut unknown = crate::project::memory::InMemoryProject::default();
    super::do_init_toml_ext(
        "widgets".to_owned(),
        "acme".to_owned(),
        "1.0.0".to_owned(),
        None,
        marker(),
        None,
        &mut unknown,
    )
    .expect("created");
    let manifest = crate::project::ProjectRead::manifest_document(&unknown)
        .expect("a manifest")
        .to_string();
    assert!(manifest.contains("sources = []"), "{manifest}");
    assert!(!manifest.contains("root"), "{manifest}");
}

/// A `.project.json` project is the two json documents, and no `model.toml`.
#[test]
fn init_json_writes_the_two_documents() {
    let mut storage = crate::project::memory::InMemoryProject::default();
    super::do_init_json(
        "widgets".to_owned(),
        "acme".to_owned(),
        "1.2.3".to_owned(),
        Some("MIT".to_owned()),
        MetaProperty::Provided(OMG_SYSML.to_owned()),
        &mut storage,
    )
    .expect("the project is created");
    assert!(storage.model_toml.is_none());
    let info = storage.info.expect("`.project.json`");
    assert_eq!(info.name, "widgets");
    assert_eq!(info.license.as_deref(), Some("MIT"));
    let meta = storage.meta.expect("`.meta.json`");
    assert_eq!(meta.metamodel.as_deref(), Some(OMG_SYSML));
}

/// A `.meta.json` has nowhere to put a marker or a Syside project's `root`.
#[test]
fn init_json_refuses_a_marker_and_a_syside_metamodel() {
    let init = |metamodel| {
        super::do_init_json(
            "widgets".to_owned(),
            "acme".to_owned(),
            "1.2.3".to_owned(),
            None,
            metamodel,
            &mut crate::project::memory::InMemoryProject::default(),
        )
    };
    assert_matches!(
        init(MetaProperty::Inherited(Inherited {
            from: InheritFrom::DefaultGroup,
            at: None,
        })),
        Err(InitError::JsonInheritedMetamodel)
    );
    assert_matches!(
        init(MetaProperty::Provided(
            "https://www.sysand.com/spec/SysML/20250201".to_owned()
        )),
        Err(InitError::JsonSysideMetamodel(_))
    );
}
