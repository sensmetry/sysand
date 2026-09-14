// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use semver::{Version, VersionReq};
use spdx::Expression;

use crate::manifest::common::{InheritFrom, Maintainer, ResolvedUrls, Sources, StdlibStem};
use crate::manifest::diagnostics::Position;
use crate::manifest::edit::render_new;
use crate::manifest::inherit::{Inherited, MetaProperty};
use crate::manifest::project::{PROJECT_KEYS, ProjectManifest, ProjectTable, Urls};
use crate::manifest::usage::{MemberUsage, Usage};
use crate::manifest::workspace_table::{GroupProperties, MemberPath, WorkspaceTable};
use crate::manifest::{ModelToml, SUPPORTED_MANIFEST_VERSION};
use crate::model::{IndexName, IndexPublisher};

const METAMODEL: &str = "https://www.omg.org/spec/SysML/20250201";

fn publisher(raw: &str) -> IndexPublisher {
    IndexPublisher::parse(raw).expect("a valid publisher")
}

fn name(raw: &str) -> IndexName {
    IndexName::parse(raw).expect("a valid name")
}

fn iri(raw: &str) -> Iri<String> {
    Iri::parse(raw.to_owned()).expect("a valid IRI")
}

fn sources(paths: &[&str]) -> Sources {
    Sources::new(paths.iter().map(|path| (*path).into()).collect()).expect("valid sources")
}

/// A `[project]` with only the three required keys, the way
/// `ProjectTable::new` leaves one.
fn built() -> ProjectTable {
    ProjectTable::new(publisher("acme"), name("widgets"), Version::new(1, 0, 0))
}

fn marker(from: InheritFrom) -> Inherited {
    Inherited { from, at: None }
}

/// Renders a manifest the way a command that writes one would.
fn render(model: &ModelToml) -> String {
    render_new(model).to_string()
}

fn project(table: ProjectTable) -> ModelToml {
    ModelToml::Project(Box::new(ProjectManifest {
        project: table,
        inline_usages: vec![],
    }))
}

fn with_usages(table: ProjectTable, inline_usages: Vec<MemberUsage>) -> ModelToml {
    ModelToml::Project(Box::new(ProjectManifest {
        project: table,
        inline_usages,
    }))
}

/// Every key of `[project]`, so that the layout of each can be read off
/// one file. `publisher` normalizes to `omg` and the metamodel is an OMG
/// one, which is what `stdlib` requires of a project that resolves.
fn maximal() -> ModelToml {
    let mut table = ProjectTable::new(publisher("OMG"), name("widgets"), Version::new(1, 0, 0));
    table.sources = Some(sources(&["Base.kerml", "Parts.sysml"]));
    table.license = Some(MetaProperty::Provided(
        Expression::parse("MIT").expect("a valid SPDX expression"),
    ));
    table.description = Some("Widgets, assorted.".to_owned());
    table.topics = Some(MetaProperty::Provided(vec![
        "widgets".to_owned(),
        "parts".to_owned(),
    ]));
    table.metamodel = Some(MetaProperty::Provided(METAMODEL.to_owned()));
    table.maintainers = Some(MetaProperty::Provided(vec![
        Maintainer::parse("Ada Lovelace <ada@example.com>").expect("a valid maintainer"),
    ]));
    table.stdlib = Some(StdlibStem::parse("KerML").expect("a valid stdlib stem"));
    table.root = true;
    table.urls = Some(MetaProperty::Provided(Urls {
        website: Some(MetaProperty::Provided(iri("https://example.com"))),
        documentation: Some(MetaProperty::Provided(iri("https://example.com/docs"))),
        repository: Some(MetaProperty::Provided(iri("https://example.com/git"))),
    }));

    with_usages(
        table,
        vec![
            MemberUsage::Declared(Usage::Index {
                publisher: publisher("acme"),
                name: name("dep"),
                version_constraint: VersionReq::parse("^1.2").expect("a valid constraint"),
            }),
            MemberUsage::Declared(Usage::SourcePath {
                publisher: publisher("acme"),
                name: name("local"),
                dir: "../local".into(),
            }),
            MemberUsage::Declared(Usage::KparPath {
                publisher: publisher("acme"),
                name: name("packed"),
                kpar_path: "vendor/packed.kpar".into(),
            }),
            MemberUsage::Declared(Usage::RemoteKpar {
                publisher: publisher("acme"),
                name: name("remote"),
                url: iri("https://example.com/remote.kpar"),
            }),
            MemberUsage::Inherited {
                publisher: publisher("acme"),
                name: name("common"),
                from: InheritFrom::DefaultGroup,
                at: None::<Position>,
            },
        ],
    )
}

const MAXIMAL: &str = r#"manifest-version = 1

[project]
publisher = "OMG"
name = "widgets"
version = "1.0.0"
sources = [
    "Base.kerml",
    "Parts.sysml",
]
license = "MIT"
description = "Widgets, assorted."
topics = ["widgets", "parts"]
metamodel = "https://www.omg.org/spec/SysML/20250201"
maintainers = ["Ada Lovelace <ada@example.com>"]
stdlib = "KerML"
root = true

[project.urls]
website = "https://example.com"
documentation = "https://example.com/docs"
repository = "https://example.com/git"

[[inline-usages]]
publisher = "acme"
name = "dep"
version = "^1.2"

[[inline-usages]]
publisher = "acme"
name = "local"
dir = "../local"

[[inline-usages]]
publisher = "acme"
name = "packed"
kpar-path = "vendor/packed.kpar"

[[inline-usages]]
publisher = "acme"
name = "remote"
url = "https://example.com/remote.kpar"

[[inline-usages]]
publisher = "acme"
name = "common"
workspace = true
"#;

#[test]
fn a_generated_project_file_has_the_documented_layout() {
    assert_eq!(render(&maximal()), MAXIMAL);
}

#[test]
fn only_the_keys_a_manifest_has_are_written() {
    // A table that has not been given its `sources`: renderable, and not
    // a manifest that resolves, since the OMG metamodel requires the
    // key even when the list is empty.
    assert_eq!(
        render(&project(built())),
        r#"manifest-version = 1

[project]
publisher = "acme"
name = "widgets"
version = "1.0.0"
"#
    );
}

#[test]
fn an_empty_sources_is_written_as_an_empty_array() {
    let mut table = built();
    table.sources = Some(sources(&[]));
    assert!(render(&project(table)).contains("\nsources = []\n"));

    let mut table = built();
    table.sources = Some(sources(&["One.kerml"]));
    assert!(render(&project(table)).contains("\nsources = [\n    \"One.kerml\",\n]\n"));
}

#[test]
fn urls_is_a_table_after_the_scalar_keys() {
    let mut table = built();
    table.urls = Some(MetaProperty::Provided(Urls {
        website: Some(MetaProperty::Provided(iri("https://example.com"))),
        ..Urls::default()
    }));

    let text = render(&project(table));

    // The failure mode is the inline form `urls = { website = ... }`,
    // which is what serializing produces: `toml_edit` writes a table's
    // values before its sub-table headers whatever order they went in,
    // so no insertion order can put `[project.urls]` in the wrong place.
    assert!(text.contains("\n[project.urls]\nwebsite = \"https://example.com\"\n"));
    assert!(!text.contains("urls = {"));
}

#[test]
fn every_usage_kind_is_written_as_it_is_read() {
    let text = render(&maximal());
    let entries: Vec<&str> = text.split("[[inline-usages]]\n").skip(1).collect();

    assert_eq!(entries.len(), 5);
    // In manifest order, each with the key that decides its kind.
    assert!(entries[0].contains("version = \"^1.2\""));
    assert!(entries[1].contains("dir = \"../local\""));
    assert!(entries[2].contains("kpar-path = \"vendor/packed.kpar\""));
    assert!(entries[3].contains("url = \"https://example.com/remote.kpar\""));
    assert!(entries[4].contains("workspace = true"));
    // An index usage carries no kind key at all.
    for absent in ["dir =", "kpar-path =", "url ="] {
        assert!(!entries[0].contains(absent));
    }
}

#[test]
fn an_inherited_key_is_written_as_a_dotted_marker() {
    let mut table = built();
    table.version = MetaProperty::Inherited(marker(InheritFrom::DefaultGroup));
    table.metamodel = Some(MetaProperty::Inherited(marker(InheritFrom::Group(
        "kerml".to_owned(),
    ))));

    let text = render(&project(table));

    assert!(text.contains("\nversion.workspace = true\n"));
    assert!(text.contains("\nmetamodel.workspace = \"kerml\"\n"));
    // Not a `[project.version]` header, which would parse as something
    // else entirely.
    assert!(!text.contains("[project.version]"));
    assert!(!text.contains("[project.metamodel]"));
}

#[test]
fn a_whole_inherited_urls_is_a_dotted_key_not_a_header() {
    let mut table = built();
    table.urls = Some(MetaProperty::Inherited(marker(InheritFrom::DefaultGroup)));

    let text = render(&project(table));

    assert!(text.contains("\nurls.workspace = true\n"));
    assert!(!text.contains("[project.urls]"));
}

#[test]
fn an_inherited_url_key_is_dotted_inside_the_urls_table() {
    let mut table = built();
    table.urls = Some(MetaProperty::Provided(Urls {
        website: Some(MetaProperty::Inherited(marker(InheritFrom::DefaultGroup))),
        repository: Some(MetaProperty::Provided(iri("https://example.com/git"))),
        ..Urls::default()
    }));

    let text = render(&project(table));

    assert!(text.contains(
        "\n[project.urls]\nwebsite.workspace = true\nrepository = \"https://example.com/git\"\n"
    ));
}

#[test]
fn root_is_written_when_it_is_set() {
    let mut table = built();
    table.root = true;
    assert!(render(&project(table)).contains("\nroot = true\n"));

    assert!(!render(&project(built())).contains("root"));
}

#[test]
fn a_generated_file_carries_no_comments() {
    assert!(!render(&maximal()).contains('#'));
}

#[test]
fn a_generated_file_states_the_supported_manifest_version() {
    assert!(render(&maximal()).starts_with(&format!(
        "manifest-version = {SUPPORTED_MANIFEST_VERSION}\n"
    )));
}

#[test]
fn every_documented_project_key_is_rendered() {
    // The guard against a key added to `ProjectTable` and to
    // `PROJECT_KEYS` that the renderer never learns to write.
    for key in PROJECT_KEYS {
        let written = if *key == "urls" {
            MAXIMAL.contains("[project.urls]")
        } else {
            MAXIMAL.contains(&format!("\n{key} = "))
        };
        assert!(written, "`{key}` is not in the generated layout");
    }
}

/// The project a generated file parses back into.
fn read_back(text: &str) -> ProjectManifest {
    ModelToml::parse("model.toml", text)
        .expect("a generated file is a valid manifest")
        .into_parts()
        .0
        .into_project("model.toml")
        .expect("a project")
}

#[test]
fn a_generated_project_file_parses_back_into_itself() {
    let text = render(&maximal());
    let read = read_back(&text);
    let table = &read.project;

    // Every key of `PROJECT_KEYS`, read back off the reparsed manifest:
    // the fixed point below cannot see a key the renderer drops, since a
    // dropped key is absent from both sides of it.
    assert_eq!(
        table.publisher.provided().map(IndexPublisher::as_str),
        Some("OMG")
    );
    assert_eq!(table.name.as_str(), "widgets");
    assert_eq!(table.version.provided(), Some(&Version::new(1, 0, 0)));
    assert_eq!(
        table.sources.as_ref().map(Sources::as_slice),
        Some(sources(&["Base.kerml", "Parts.sysml"]).as_slice())
    );
    assert_eq!(
        table
            .license
            .as_ref()
            .and_then(MetaProperty::provided)
            .map(ToString::to_string),
        Some("MIT".to_owned())
    );
    assert_eq!(table.description.as_deref(), Some("Widgets, assorted."));
    assert_eq!(
        table.topics.as_ref().and_then(MetaProperty::provided),
        Some(&vec!["widgets".to_owned(), "parts".to_owned()])
    );
    assert_eq!(
        table
            .metamodel
            .as_ref()
            .and_then(MetaProperty::provided)
            .map(String::as_str),
        Some(METAMODEL)
    );
    assert_eq!(
        table
            .maintainers
            .as_ref()
            .and_then(MetaProperty::provided)
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        table.stdlib.as_ref().map(ToString::to_string),
        Some("KerML".to_owned())
    );
    assert!(table.root);
    assert!(
        table
            .urls
            .as_ref()
            .and_then(MetaProperty::provided)
            .is_some()
    );
    assert_eq!(read.inline_usages.len(), 5);

    // The fixed point: nothing shifts on a second pass.
    assert_eq!(render(&ModelToml::Project(Box::new(read))), text);
}

const KERML: &str = "https://www.omg.org/spec/KerML/20250201";

fn member(raw: &str) -> MemberPath {
    MemberPath {
        raw: raw.to_owned(),
        path: crate::utils::normalize_relative(raw).expect("a valid member path"),
    }
}

fn index_usage(project: &str, constraint: &str) -> Usage {
    Usage::Index {
        publisher: publisher("acme"),
        name: name(project),
        version_constraint: VersionReq::parse(constraint).expect("a valid constraint"),
    }
}

/// A workspace with membership lists, a default group and a named one.
fn workspace() -> ModelToml {
    let default_group = GroupProperties {
        publisher: Some(publisher("acme")),
        version: Some(Version::new(1, 0, 0)),
        license: Some(Expression::parse("MIT").expect("a valid SPDX expression")),
        topics: Some(vec!["widgets".to_owned()]),
        metamodel: Some(METAMODEL.to_owned()),
        maintainers: Some(vec![
            Maintainer::parse("Ada Lovelace <ada@example.com>").expect("a valid maintainer"),
        ]),
        urls: Some(ResolvedUrls {
            website: Some(iri("https://example.com")),
            ..ResolvedUrls::default()
        }),
        inline_usages: vec![index_usage("dep", "^1.2")],
    };
    let kerml = GroupProperties {
        metamodel: Some(KERML.to_owned()),
        inline_usages: vec![index_usage("other", "^2")],
        ..GroupProperties::default()
    };

    ModelToml::Workspace(Box::new(WorkspaceTable {
        members: vec![member("app"), member("lib")],
        exclude: vec![member("scratch")],
        default_group,
        groups: std::collections::BTreeMap::from([("kerml".to_owned(), kerml)]),
    }))
}

const WORKSPACE: &str = r#"manifest-version = 1

[workspace]
members = ["app", "lib"]
exclude = ["scratch"]
publisher = "acme"
version = "1.0.0"
license = "MIT"
topics = ["widgets"]
metamodel = "https://www.omg.org/spec/SysML/20250201"
maintainers = ["Ada Lovelace <ada@example.com>"]

[workspace.urls]
website = "https://example.com"

[[workspace.inline-usages]]
publisher = "acme"
name = "dep"
version = "^1.2"

[workspace.group.kerml]
metamodel = "https://www.omg.org/spec/KerML/20250201"

[[workspace.group.kerml.inline-usages]]
publisher = "acme"
name = "other"
version = "^2"
"#;

#[test]
fn a_generated_workspace_file_has_the_documented_layout() {
    // model-toml.md fixes the project layout in detail and says of this
    // one only that a workspace file "has `manifest-version` and then
    // `[workspace]`". The order below is Sysand's own: the membership
    // lists, then the default group's keys in the order the reader
    // recognizes them.
    assert_eq!(render(&workspace()), WORKSPACE);
}

#[test]
fn a_named_group_is_written_under_its_own_header() {
    let text = render(&workspace());

    assert!(text.contains(
        "\n[workspace.group.kerml]\nmetamodel = \"https://www.omg.org/spec/KerML/20250201\"\n"
    ));
    assert!(text.contains("\n[[workspace.group.kerml.inline-usages]]\n"));
    // `[workspace.group]` holds nothing of its own and gets no header.
    assert!(!text.contains("\n[workspace.group]\n"));
}

#[test]
fn a_generated_workspace_file_parses_back_into_itself() {
    let text = render(&workspace());
    let table = ModelToml::parse("model.toml", &text)
        .expect("a generated workspace file is a valid manifest")
        .into_parts()
        .0
        .into_workspace("model.toml")
        .expect("a workspace");

    assert_eq!(
        table
            .members
            .iter()
            .map(|m| m.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["app", "lib"]
    );
    assert_eq!(
        table
            .exclude
            .iter()
            .map(|m| m.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["scratch"]
    );
    assert_eq!(
        table
            .default_group
            .publisher
            .as_ref()
            .map(IndexPublisher::as_str),
        Some("acme")
    );
    assert_eq!(table.default_group.version, Some(Version::new(1, 0, 0)));
    assert_eq!(table.default_group.inline_usages.len(), 1);
    assert!(table.default_group.urls.is_some());
    assert_eq!(table.groups.len(), 1);
    assert_eq!(table.groups["kerml"].inline_usages.len(), 1);

    // The fixed point: nothing shifts on a second pass.
    assert_eq!(render(&ModelToml::Workspace(Box::new(table))), text);
}
