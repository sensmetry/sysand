// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;

use crate::commands::init::{
    GivenProject, InheritableKey, InitError, InitProject, RequiredKey, do_init_json,
    do_init_memory, do_init_toml,
};
use crate::manifest::common::InheritFrom;
use crate::manifest::workspace_config::{GroupProperties, WorkspaceConfig};
use crate::model::{KnownMetamodel, ProjectName, ProjectPublisher};
use crate::project::memory::InMemoryError;
use crate::project::{ProjectRead as _, memory::InMemoryProject};

const OMG_SYSML: &str = "https://www.omg.org/spec/SysML/20250201";
const SYSIDE_SYSML: &str = "https://www.sysand.com/spec/SysML/20250201";

fn init(metamodel: &str) -> String {
    do_init_memory("widgets", "acme", "1.2.3", None, metamodel.to_owned())
        .expect("the project is created")
        .model_toml
        .expect("a manifest")
}

/// A project outside any workspace, from the keys as written.
fn given(
    name: &str,
    publisher: &str,
    version: &str,
    license: Option<&str>,
    metamodel: &str,
) -> Result<GivenProject, InitError<InMemoryError>> {
    GivenProject::parse(
        name.to_owned(),
        publisher.to_owned(),
        version.to_owned(),
        license.map(str::to_owned),
        metamodel.to_owned(),
    )
}

fn publisher(value: &str) -> ProjectPublisher {
    ProjectPublisher::parse(value.to_owned()).expect("a valid publisher")
}

fn metamodel(value: &str) -> KnownMetamodel {
    KnownMetamodel::recognize(value).expect("a recognized metamodel")
}

fn license(value: &str) -> spdx::Expression {
    spdx::Expression::parse(value).expect("a valid license")
}

/// A project that gives only its name, created in `workspace`.
fn bare(workspace: Option<&WorkspaceConfig>) -> InitProject<'_> {
    InitProject {
        name: ProjectName::parse("widgets".to_owned()).expect("a valid name"),
        publisher: None,
        version: None,
        license: None,
        metamodel: None,
        workspace,
    }
}

fn init_ext(init: InitProject<'_>) -> Result<String, InitError<InMemoryError>> {
    let mut storage = InMemoryProject::default();
    do_init_toml(init, &mut storage)?;
    Ok(storage.manifest_document().expect("a manifest").to_string())
}

/// A workspace whose default group gives every inheritable key `init`
/// writes.
fn full_workspace() -> WorkspaceConfig {
    let mut workspace = WorkspaceConfig::default();
    let group = &mut workspace.default_group;
    group.publisher = Some(crate::model::IndexPublisher::parse("acme".to_owned()).expect("valid"));
    group.version = Some(semver::Version::new(1, 0, 0));
    group.license = Some(spdx::Expression::parse("MIT").expect("valid"));
    group.metamodel = KnownMetamodel::recognize(SYSIDE_SYSML);
    workspace
}

/// A project declares a metamodel, and `init` is given it.
#[test]
fn a_metamodel_is_written_when_given() {
    let written = init(OMG_SYSML);
    assert!(
        written.contains(&format!("\nmetamodel = \"{OMG_SYSML}\"\n")),
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
        "https://example.com/spec".to_owned(),
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

/// Every key left out is inherited from the default group that gives it,
/// and the layout follows the group's metamodel.
#[test]
fn init_writes_a_marker_for_every_key_the_group_gives() {
    let workspace = full_workspace();
    let manifest = init_ext(bare(Some(&workspace))).expect("created");
    for key in ["publisher", "version", "license", "metamodel"] {
        assert!(
            manifest.contains(&format!("\n{key}.workspace = true\n")),
            "{key}: {manifest}"
        );
    }
    assert!(manifest.contains("root = \"widgets.sysml\""), "{manifest}");
}

/// A given key wins over the group's.
#[test]
fn a_given_key_is_written_even_when_the_group_gives_it() {
    let workspace = full_workspace();
    let manifest = init_ext(InitProject {
        publisher: Some(publisher("bravo")),
        version: Some(semver::Version::new(2, 0, 0)),
        license: Some(license("Apache-2.0")),
        metamodel: Some(metamodel(OMG_SYSML)),
        ..bare(Some(&workspace))
    })
    .expect("created");
    assert!(!manifest.contains(".workspace"), "{manifest}");
    assert!(manifest.contains("version = \"2.0.0\""), "{manifest}");
    assert!(manifest.contains("sources = []"), "{manifest}");
}

/// Without a group that gives them, `version` defaults and `license` is
/// left out; a missing `metamodel` or `publisher` is refused, the
/// metamodel first.
#[test]
fn a_key_the_group_does_not_give_takes_its_fallback() {
    let empty = WorkspaceConfig::default();
    for workspace in [None, Some(&empty)] {
        let error = init_ext(bare(workspace)).expect_err("no metamodel");
        assert_matches!(error, InitError::Missing(RequiredKey::Metamodel));

        let error = init_ext(InitProject {
            metamodel: Some(metamodel(OMG_SYSML)),
            ..bare(workspace)
        })
        .expect_err("no publisher");
        assert_matches!(error, InitError::Missing(RequiredKey::Publisher));

        let manifest = init_ext(InitProject {
            publisher: Some(publisher("acme")),
            metamodel: Some(metamodel(OMG_SYSML)),
            ..bare(workspace)
        })
        .expect("created");
        assert!(manifest.contains("version = \"0.0.1\""), "{manifest}");
        assert!(!manifest.contains("license"), "{manifest}");
        assert!(!manifest.contains(".workspace"), "{manifest}");
    }
}

/// A key only a named group gives is inherited from that group, and the
/// layout follows that group's metamodel.
#[test]
fn a_key_only_a_named_group_gives_is_inherited_from_it() {
    let mut workspace = WorkspaceConfig::default();
    workspace.default_group.version = Some(semver::Version::new(1, 0, 0));
    let syside = GroupProperties {
        publisher: Some(crate::model::IndexPublisher::parse("acme".to_owned()).expect("valid")),
        metamodel: KnownMetamodel::recognize(SYSIDE_SYSML),
        ..GroupProperties::default()
    };
    workspace.groups.insert("syside".to_owned(), syside);
    workspace
        .groups
        .insert("empty".to_owned(), GroupProperties::default());

    let manifest = init_ext(bare(Some(&workspace))).expect("created");
    for key in ["publisher", "metamodel"] {
        assert!(
            manifest.contains(&format!("\n{key}.workspace = \"syside\"\n")),
            "{key}: {manifest}"
        );
    }
    assert!(
        manifest.contains("\nversion.workspace = true\n"),
        "{manifest}"
    );
    assert!(!manifest.contains("license"), "{manifest}");
    assert!(manifest.contains("root = \"widgets.sysml\""), "{manifest}");
}

/// A key left out that more than one group gives is refused, naming the
/// groups, default group first.
#[test]
fn a_key_more_than_one_group_gives_is_refused() {
    let mut workspace = full_workspace();
    let license = GroupProperties {
        license: Some(spdx::Expression::parse("Apache-2.0").expect("valid")),
        ..GroupProperties::default()
    };
    workspace.groups.insert("b".to_owned(), license.clone());
    workspace.groups.insert("a".to_owned(), license);

    let error = init_ext(bare(Some(&workspace))).expect_err("ambiguous");
    assert_matches!(
        &error,
        InitError::AmbiguousGroup { key: InheritableKey::License, groups }
            if *groups == [
                InheritFrom::DefaultGroup,
                InheritFrom::Group("a".to_owned()),
                InheritFrom::Group("b".to_owned()),
            ],
        "unexpected error: {error}"
    );
    assert!(
        error
            .to_string()
            .contains("(the default group, group `a`, group `b`)"),
        "{error}"
    );

    // Two named groups, without the default group, are ambiguous too.
    workspace.default_group.license = None;
    assert_matches!(
        init_ext(bare(Some(&workspace))),
        Err(InitError::AmbiguousGroup {
            key: InheritableKey::License,
            groups: _
        })
    );
}

/// A given key is not inherited, so groups that disagree about it do not
/// matter.
#[test]
fn a_given_key_is_not_ambiguous() {
    let mut workspace = full_workspace();
    workspace.groups.insert(
        "other".to_owned(),
        GroupProperties {
            license: Some(spdx::Expression::parse("Apache-2.0").expect("valid")),
            ..GroupProperties::default()
        },
    );
    let manifest = init_ext(InitProject {
        license: Some(license("MIT")),
        ..bare(Some(&workspace))
    })
    .expect("created");
    assert!(manifest.contains("\nlicense = \"MIT\"\n"), "{manifest}");
}

/// `init` with a Syside metamodel writes the Syside layout and creates the root file, through
/// the same storage the manifest goes to.
#[test]
fn init_with_a_syside_metamodel_writes_the_layout_and_the_root_file() {
    let project = do_init_memory(
        "My Widgets",
        "acme",
        "1.0.0",
        None,
        "https://www.sysand.com/spec/KerML/20250201".to_owned(),
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
    let mut storage = InMemoryProject::default();
    storage
        .files
        .insert("widgets.sysml".into(), "package Widgets { }\n".to_owned());
    let project = given("widgets", "acme", "1.0.0", None, SYSIDE_SYSML).expect("valid");
    do_init_toml(project.into(), &mut storage).expect("the project is created");
    assert_eq!(
        storage
            .files
            .get(&typed_path::Utf8UnixPathBuf::from("widgets.sysml")),
        Some(&"package Widgets { }\n".to_owned())
    );
}

/// A `.project.json` project is the two json documents, and no `model.toml`.
#[test]
fn init_json_writes_the_two_documents() {
    let mut storage = InMemoryProject::default();
    let project = given("widgets", "acme", "1.2.3", Some("MIT"), OMG_SYSML).expect("valid");
    do_init_json(project, &mut storage).expect("the project is created");
    assert!(storage.model_toml.is_none());
    let info = storage.info.expect("`.project.json`");
    assert_eq!(info.name, "widgets");
    assert_eq!(info.license.as_deref(), Some("MIT"));
    let meta = storage.meta.expect("`.meta.json`");
    assert_eq!(meta.metamodel.as_deref(), Some(OMG_SYSML));
}

/// A `.project.json` takes any publisher and name the json reader accepts,
/// including ones an index usage refuses.
#[test]
fn init_json_takes_the_json_identifier_rule() {
    let mut storage = InMemoryProject::default();
    let project = given("my_widgets", "ac", "1.2.3", None, OMG_SYSML).expect("valid");
    do_init_json(project, &mut storage).expect("the project is created");
    let info = storage.info.expect("`.project.json`");
    assert_eq!(info.name, "my_widgets");
    assert_eq!(info.publisher.as_deref(), Some("ac"));

    assert_matches!(
        given("a/b", "acme", "1.2.3", None, OMG_SYSML),
        Err(InitError::ProjectField(value, _)) if &*value == "a/b"
    );
}

/// A `.meta.json` has nowhere to put a Syside project's `root`.
#[test]
fn init_json_refuses_a_syside_metamodel() {
    let project = given("widgets", "acme", "1.2.3", None, SYSIDE_SYSML).expect("valid");
    assert_matches!(
        do_init_json(project, &mut InMemoryProject::default()),
        Err(InitError::JsonSysideMetamodel(_))
    );
}
