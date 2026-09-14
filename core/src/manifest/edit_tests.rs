// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use semver::{Version, VersionReq};
use spdx::Expression;
use toml_edit::{DocumentMut, Value};
use typed_path::Utf8UnixPathBuf;

use crate::manifest::common::{
    InheritFrom, Maintainer, ResolvedUrls, Sources, SourcesError, StdlibStem,
};
use crate::manifest::diagnostics::Position;
use crate::manifest::edit::{
    EditError, INHERITABLE_KEYS, ProjectKey, clear_project_key, clear_url_key, exclude_source,
    include_source, open, render_new, set_inherit_marker, set_project_key, set_url_key,
};
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

// ---------------------------------------------------------------------
// In-place editing. model-toml.md "Editing the manifest": every command
// that writes a `model.toml` "edits the existing document in place,
// preserving comments, key order and formatting; comments are the
// reason a TOML manifest exists, so no writer may drop them."
// ---------------------------------------------------------------------

/// A manifest with a comment in every place a comment can be.
const COMMENTED: &str = r#"manifest-version = 1

# what this project is
[project]
# who publishes it
publisher = "acme"  # the publisher
name = "widgets"
version = "1.0.0"
sources = [  # the list
    # the base
    "Base.kerml",  # first
    "Parts.sysml",
]

[project.urls]
website = "https://example.com"  # the site

# the dependency we vendor
[[inline-usages]]
publisher = "acme"
name = "dep"
version = "^1"

# a note at the end
"#;

fn open_commented() -> DocumentMut {
    open("model.toml", COMMENTED).expect("the fixture is valid TOML")
}

fn opened(text: &str) -> DocumentMut {
    open("model.toml", text).expect("the fixture is valid TOML")
}

#[test]
fn a_file_that_is_not_toml_is_refused_the_way_reading_refuses_it() {
    let broken = "manifest-version = 1\n[project\n";

    let editing = open("model.toml", broken).expect_err("not valid TOML");
    let reading = ModelToml::parse("model.toml", broken).expect_err("not valid TOML");

    assert_eq!(editing.message, reading.diagnostics()[0].message);
    assert_eq!(
        editing.position.map(|at| (at.line, at.column)),
        reading.diagnostics()[0]
            .position
            .as_ref()
            .map(|at| (at.line, at.column))
    );
}

#[test]
fn setting_a_key_keeps_the_comment_after_its_value() {
    let mut doc = open_commented();
    set_project_key(&mut doc, ProjectKey::Publisher, Value::from("other"))
        .expect("publisher is settable");

    assert!(
        doc.to_string()
            .contains("publisher = \"other\"  # the publisher\n")
    );
}

#[test]
fn setting_a_key_changes_one_line_and_nothing_else() {
    let mut doc = open_commented();
    // The hardest key in the fixture: a comment above it and one after
    // its value, so this guards the decor transplant as well as the
    // order and whitespace of every line it does not touch.
    set_project_key(&mut doc, ProjectKey::Publisher, Value::from("other")).expect("settable");

    // "comments, key order and formatting" is a property of the file,
    // not of one line: every other byte is unchanged. `replacen`
    // because the usage entry names a publisher too.
    assert_eq!(
        doc.to_string(),
        COMMENTED.replacen("publisher = \"acme\"", "publisher = \"other\"", 1)
    );
}

#[test]
fn setting_a_key_that_is_not_there_appends_it_inside_the_table() {
    let mut doc = open_commented();
    set_project_key(&mut doc, ProjectKey::Description, Value::from("Widgets.")).expect("settable");

    let text = doc.to_string();
    // Inside `[project]`: after its last scalar, before the first
    // sub-table header.
    assert!(text.contains("]\ndescription = \"Widgets.\"\n\n[project.urls]"));
}

#[test]
fn an_edit_keeps_keys_in_the_order_the_file_wrote_them() {
    let scrambled = "manifest-version = 1\n\n[project]\nversion = \"1.0.0\"\nname = \"widgets\"\npublisher = \"acme\"\n";
    let mut doc = opened(scrambled);
    set_project_key(&mut doc, ProjectKey::Name, Value::from("gadgets")).expect("settable");

    assert_eq!(
        doc.to_string(),
        scrambled.replace("name = \"widgets\"", "name = \"gadgets\"")
    );
}

#[test]
fn setting_a_key_twice_is_the_same_as_setting_it_once() {
    let mut once = open_commented();
    set_project_key(&mut once, ProjectKey::Publisher, Value::from("other")).expect("settable");

    let mut twice = open_commented();
    set_project_key(&mut twice, ProjectKey::Publisher, Value::from("first")).expect("settable");
    set_project_key(&mut twice, ProjectKey::Publisher, Value::from("other")).expect("settable");

    assert_eq!(twice.to_string(), once.to_string());
}

#[test]
fn clearing_a_key_says_whether_it_was_there() {
    let mut doc = open_commented();

    assert_eq!(clear_project_key(&mut doc, ProjectKey::Publisher), Ok(true));
    assert_eq!(
        clear_project_key(&mut doc, ProjectKey::Publisher),
        Ok(false)
    );
    assert!(
        !doc.to_string()
            .contains("publisher = \"acme\"  # the publisher")
    );
    // The comment that introduced the table is not the key's.
    assert!(
        doc.to_string()
            .contains("# what this project is\n[project]\n")
    );
}

#[test]
fn a_url_key_is_set_inside_a_project_urls_table() {
    let mut doc = open_commented();
    set_url_key(&mut doc, "repository", "https://example.com/git").expect("a url key");

    let text = doc.to_string();
    assert!(text.contains(
        "[project.urls]\nwebsite = \"https://example.com\"  # the site\nrepository = \"https://example.com/git\"\n"
    ));
}

#[test]
fn a_url_key_is_set_inside_an_inline_urls_table() {
    // `urls` may be an inline or a standard table; users write both.
    let inline = "manifest-version = 1\n\n[project]\nname = \"w\"\nurls = { website = \"https://a\" }  # links\n";
    let mut doc = opened(inline);
    set_url_key(&mut doc, "documentation", "https://d").expect("a url key");

    let text = doc.to_string();
    assert!(text.contains("documentation = \"https://d\""));
    assert!(text.contains("# links"));
    assert!(text.contains("urls = {"), "the table stays inline");
}

#[test]
fn a_url_key_is_set_beside_a_dotted_urls_key() {
    let dotted = "manifest-version = 1\n\n[project]\nname = \"w\"\nurls.website = \"https://a\"  # the site\n";
    let mut doc = opened(dotted);
    set_url_key(&mut doc, "repository", "https://r").expect("a url key");

    let text = doc.to_string();
    assert!(text.contains("urls.website = \"https://a\"  # the site\n"));
    assert!(text.contains("urls.repository = \"https://r\"\n"));
}

#[test]
fn setting_a_url_key_creates_a_header_table_not_an_inline_one() {
    let bare = "manifest-version = 1\n\n[project]\nname = \"w\"\n";
    let mut doc = opened(bare);
    set_url_key(&mut doc, "website", "https://a").expect("a url key");

    let text = doc.to_string();
    assert!(text.contains("[project.urls]\nwebsite = \"https://a\"\n"));
    assert!(!text.contains("urls = {"));
}

#[test]
fn clearing_the_last_url_key_leaves_the_container() {
    // Removing the table too is a second edit the caller did not ask
    // for, and it would take the comments in it.
    let mut doc = open_commented();
    assert_eq!(clear_url_key(&mut doc, "website"), Ok(true));

    let text = doc.to_string();
    assert!(text.contains("[project.urls]"));
    assert!(!text.contains("website = "));
}

#[test]
fn a_marker_replaces_a_value_and_keeps_both_its_comments() {
    let commented = "manifest-version = 1\n\n[project]\n# which version\nversion = \"1.0.0\"  # pinned\nname = \"w\"\n";
    let mut doc = opened(commented);
    set_inherit_marker(&mut doc, "version", &InheritFrom::DefaultGroup).expect("inheritable");

    assert_eq!(
        doc.to_string(),
        "manifest-version = 1\n\n[project]\n# which version\nversion.workspace = true  # pinned\nname = \"w\"\n"
    );
}

#[test]
fn a_value_replaces_a_marker_and_keeps_both_its_comments() {
    let inherited = "manifest-version = 1\n\n[project]\n# which version\nversion.workspace = true  # inherited\nname = \"w\"\n";
    let mut doc = opened(inherited);
    set_project_key(&mut doc, ProjectKey::Version, Value::from("2.0.0")).expect("settable");

    assert_eq!(
        doc.to_string(),
        "manifest-version = 1\n\n[project]\n# which version\nversion = \"2.0.0\"  # inherited\nname = \"w\"\n"
    );
}

#[test]
fn inheriting_a_whole_urls_table_loses_the_comments_inside_it() {
    // Inherent: the table those comments annotate no longer exists.
    let mut doc = open_commented();
    set_inherit_marker(&mut doc, "urls", &InheritFrom::DefaultGroup).expect("inheritable");

    let text = doc.to_string();
    assert!(text.contains("urls.workspace = true"));
    assert!(!text.contains("[project.urls]"));
    assert!(!text.contains("# the site"));
}

#[test]
fn the_inheritable_keys_are_the_ones_the_reader_accepts_a_marker_for() {
    // `edit.rs` has to name the inheritable keys, and `project.rs`
    // decides them one `inheritable(..)` call at a time. This is what
    // keeps the two from drifting.
    for key in PROJECT_KEYS {
        // The marker takes the place of the key's own value: writing
        // both is a duplicate key, which is a different refusal.
        let required: String = [
            ("publisher", "publisher = \"acme\"\n"),
            ("name", "name = \"widgets\"\n"),
            ("version", "version = \"1.0.0\"\n"),
            ("sources", "sources = []\n"),
        ]
        .into_iter()
        .filter(|(name, _)| name != key)
        .map(|(_, line)| line)
        .collect();
        let text = format!("manifest-version = 1\n\n[project]\n{required}{key}.workspace = true\n");
        let accepted = ModelToml::parse("model.toml", &text).is_ok();
        assert_eq!(
            accepted,
            INHERITABLE_KEYS.contains(key),
            "`{key}`: the reader and `INHERITABLE_KEYS` disagree"
        );
    }
}

#[test]
fn a_key_that_cannot_be_inherited_is_refused_a_marker() {
    let mut doc = open_commented();

    assert_eq!(
        set_inherit_marker(&mut doc, "name", &InheritFrom::DefaultGroup),
        Err(EditError::NotInheritable("name".to_owned()))
    );
}

#[test]
fn editing_a_workspace_file_as_a_project_is_refused() {
    let workspace = "manifest-version = 1\n\n[workspace]\nmembers = [\"app\"]\n";
    let mut doc = opened(workspace);

    assert_eq!(
        set_project_key(&mut doc, ProjectKey::Name, Value::from("w")),
        Err(EditError::NotAProject)
    );
}

// --- `include` and `exclude` ------------------------------------------

/// The `sources` array of an edited document, as text. The fixtures
/// have no other `]`, so the first one closes it.
fn sources_of(doc: &DocumentMut) -> String {
    let text = doc.to_string();
    let start = text.find("sources = ").expect("a sources array");
    let end = start + text[start..].find(']').expect("a closing bracket") + 1;
    text[start..end].to_owned()
}

/// A manifest that is complete apart from the `sources` array under
/// test, so that an edited one can be read back.
fn with_sources(array: &str) -> String {
    format!(
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = {array}\n"
    )
}

#[test]
fn including_a_path_appends_it_to_sources() {
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\",\n]"));
    let added = include_source(&mut doc, "Parts.sysml").expect("a new source");

    assert_eq!(added, Utf8UnixPathBuf::from("Parts.sysml"));
    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn including_normalizes_the_path_as_reading_does() {
    let mut doc = opened(&with_sources("[]"));
    let added = include_source(&mut doc, "./src/../Parts.sysml").expect("a new source");

    assert_eq!(added, Utf8UnixPathBuf::from("Parts.sysml"));
    assert!(sources_of(&doc).contains("\"Parts.sysml\""));
}

#[test]
fn including_a_path_that_is_already_listed_is_refused() {
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\",\n]"));

    // Normalized, so `./Base.kerml` is the same entry.
    let error = include_source(&mut doc, "./Base.kerml").expect_err("a duplicate");
    assert!(matches!(
        error,
        EditError::Sources(SourcesError::Duplicate { .. })
    ));
}

#[test]
fn including_keeps_a_multi_line_array_multi_line() {
    let mut doc = opened(&with_sources("[\n    # the base\n    \"Base.kerml\",\n]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    # the base\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn including_moves_the_arrays_trailing_comment_onto_the_new_entry() {
    // A comment after the last entry is held in the array's trailing,
    // so a naive push would move it below the entry it describes.
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\",  # the base\n]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",  # the base\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn including_into_an_array_with_no_trailing_comma_stays_valid() {
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\"\n]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");

    let text = doc.to_string();
    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]"
    );
    ModelToml::parse("model.toml", &text).expect("the edited file is still a manifest");
}

#[test]
fn including_into_an_empty_array_writes_one_entry() {
    // A generated file then edited: `render_new` writes `sources = []`.
    let mut doc = opened(&with_sources("[]"));
    include_source(&mut doc, "Base.kerml").expect("a new source");

    assert_eq!(sources_of(&doc), "sources = [\"Base.kerml\"]");
}

#[test]
fn including_into_a_single_line_array_stays_on_one_line() {
    let mut doc = opened(&with_sources("[\"Base.kerml\"]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\"Base.kerml\", \"Parts.sysml\"]"
    );
}

#[test]
fn including_keeps_an_indented_closing_bracket_indented() {
    let mut doc = opened(&with_sources("[\n        \"Base.kerml\",\n  ]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n        \"Base.kerml\",\n        \"Parts.sysml\",\n  ]"
    );
}

#[test]
fn including_twice_appends_two_entries() {
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\",\n]"));
    include_source(&mut doc, "Parts.sysml").expect("a new source");
    include_source(&mut doc, "Links.kerml").expect("a new source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",\n    \"Parts.sysml\",\n    \"Links.kerml\",\n]"
    );
}

#[test]
fn excluding_removes_the_entry_and_the_comment_on_its_line() {
    let mut doc = opened(&with_sources(
        "[\n    \"Base.kerml\",  # first\n    \"Parts.sysml\",  # second\n    \"Links.kerml\",\n]",
    ));
    let removed = exclude_source(&mut doc, "Parts.sysml").expect("a listed source");

    assert_eq!(removed, Utf8UnixPathBuf::from("Parts.sysml"));
    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",  # first\n    \"Links.kerml\",\n]"
    );
}

#[test]
fn excluding_keeps_the_comment_that_belongs_to_the_previous_entry() {
    // A TOML comment binds forward: `# first` is written on Base's line
    // but stored at the front of Parts's prefix. Removing Parts must
    // not take it -- it describes a file that is still listed.
    let mut doc = opened(&with_sources(
        "[\n    \"Base.kerml\",  # first\n    \"Parts.sysml\",\n]",
    ));
    exclude_source(&mut doc, "Parts.sysml").expect("a listed source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",  # first\n]"
    );
}

#[test]
fn excluding_the_first_entry_keeps_the_comment_on_the_bracket_line() {
    let mut doc = opened(&with_sources(
        "[  # the list\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]",
    ));
    exclude_source(&mut doc, "Base.kerml").expect("a listed source");

    assert_eq!(
        sources_of(&doc),
        "sources = [  # the list\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn excluding_keeps_a_comment_block_above_the_next_entry() {
    let mut doc = opened(&with_sources(
        "[\n    \"Base.kerml\",\n    # everything about parts\n    # on two lines\n    \"Parts.sysml\",\n]",
    ));
    exclude_source(&mut doc, "Base.kerml").expect("a listed source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    # everything about parts\n    # on two lines\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn excluding_the_last_entry_takes_its_comment_with_it() {
    // The last entry's own comment lives in the array's trailing.
    // Leaving it there would re-bind it to the entry before.
    let mut doc = opened(&with_sources(
        "[\n    \"Base.kerml\",  # first\n    \"Parts.sysml\",  # last one\n]",
    ));
    exclude_source(&mut doc, "Parts.sysml").expect("a listed source");

    assert_eq!(
        sources_of(&doc),
        "sources = [\n    \"Base.kerml\",  # first\n]"
    );
}

#[test]
fn excluding_the_only_entry_leaves_an_empty_array() {
    let mut doc = opened(&with_sources("[\n    \"Base.kerml\",  # only\n]"));
    exclude_source(&mut doc, "Base.kerml").expect("a listed source");

    assert_eq!(sources_of(&doc), "sources = [\n]");
}

#[test]
fn excluding_from_a_single_line_array_leaves_no_gap() {
    let mut doc = opened(&with_sources("[\"Base.kerml\", \"Parts.sysml\"]"));
    exclude_source(&mut doc, "Parts.sysml").expect("a listed source");

    assert_eq!(sources_of(&doc), "sources = [\"Base.kerml\"]");
}

#[test]
fn excluding_a_path_that_is_not_listed_is_refused() {
    let mut doc = opened(&with_sources("[\"Base.kerml\"]"));

    assert_eq!(
        exclude_source(&mut doc, "Parts.sysml"),
        Err(EditError::SourceNotListed(Utf8UnixPathBuf::from(
            "Parts.sysml"
        )))
    );
}

#[test]
fn including_and_excluding_a_manifest_with_no_sources_is_refused() {
    let bare = "manifest-version = 1\n\n[project]\nname = \"widgets\"\n";

    assert_eq!(
        include_source(&mut opened(bare), "Base.kerml"),
        Err(EditError::NoSources)
    );
    assert_eq!(
        exclude_source(&mut opened(bare), "Base.kerml"),
        Err(EditError::NoSources)
    );
}

#[test]
fn including_into_a_sources_that_is_already_invalid_says_which_entry() {
    // `Sources::new` judges the whole array, so an entry the user did
    // not touch is what the message names. One reading of the rule,
    // not two.
    let mut doc = opened(&with_sources("[\"README.md\"]"));

    let error = include_source(&mut doc, "Base.kerml").expect_err("the array is invalid");
    assert_eq!(
        error,
        EditError::Sources(SourcesError::NotModelFile("README.md".to_owned()))
    );
}
