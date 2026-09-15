// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use typed_path::Utf8UnixPathBuf;

use super::{IncludeError, do_include};
use crate::commands::exclude::{ExcludeError, do_exclude};
use crate::project::{ProjectMut as _, ProjectRead as _, memory::InMemoryProject};

const MANIFEST: &str = "manifest-version = 1\n\n\
                        [project]\n\
                        publisher = \"acme\"\n\
                        name = \"widgets\"\n\
                        version = \"1.0.0\"\n\
                        # the files this project is made of\n\
                        sources = [\n    \"Base.kerml\",\n]\n";

/// A `model.toml` project holding `Base.kerml` and one more file, so a
/// `sources` entry can name something that is really there.
fn toml_project() -> InMemoryProject {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&MANIFEST.parse().expect("a manifest"), false)
        .expect("stores a manifest");
    project
        .files
        .insert("Base.kerml".into(), "package Base;\n".to_owned());
    project
        .files
        .insert("Parts.sysml".into(), "package Parts;\n".to_owned());
    project
}

fn sources_of(project: &InMemoryProject) -> String {
    let text = project.model_toml.clone().expect("a manifest");
    let start = text.find("sources = ").expect("a sources array");
    let end = start + text[start..].find(']').expect("a closing bracket") + 1;
    text[start..end].to_owned()
}

fn include(
    project: &mut InMemoryProject,
    path: &str,
) -> Result<(), IncludeError<crate::project::memory::InMemoryError>> {
    do_include(
        project,
        [Utf8UnixPathBuf::from(path)].into_iter(),
        false,
        true,
        None,
    )
}

#[test]
fn including_a_source_appends_it_to_the_manifest() {
    let mut project = toml_project();
    include(&mut project, "Parts.sysml").expect("a new source");

    assert_eq!(
        sources_of(&project),
        "sources = [\n    \"Base.kerml\",\n    \"Parts.sysml\",\n]"
    );
}

#[test]
fn including_keeps_the_comments_in_the_manifest() {
    let mut project = toml_project();
    include(&mut project, "Parts.sysml").expect("a new source");

    assert!(
        project
            .model_toml
            .as_deref()
            .expect("a manifest")
            .contains("# the files this project is made of")
    );
}

#[test]
fn including_writes_no_metadata() {
    // model-toml.md: "Neither extracts symbols or hashes files, as both
    // maps are computed at build."
    let mut project = toml_project();
    include(&mut project, "Parts.sysml").expect("a new source");

    assert!(project.info.is_none());
    assert!(project.meta.is_none());
}

#[test]
fn including_a_duplicate_is_refused() {
    let mut project = toml_project();

    // Normalized, so `./Base.kerml` is the entry that is already there.
    let error = include(&mut project, "./Base.kerml").expect_err("a duplicate");
    assert!(matches!(error, IncludeError::Edit(_)));
}

#[test]
fn including_a_path_that_is_not_a_model_file_is_refused() {
    let mut project = toml_project();

    let error = include(&mut project, "README.md").expect_err("not a model file");
    assert!(error.to_string().contains("README.md"));
}

#[test]
fn compute_checksum_is_refused_on_a_toml_project() {
    let mut project = toml_project();

    let error = do_include(
        &mut project,
        [Utf8UnixPathBuf::from("Parts.sysml")].into_iter(),
        true,
        true,
        None,
    )
    .expect_err("a computed field");

    assert!(matches!(error, IncludeError::ComputedField("checksum")));
    assert!(
        error
            .to_string()
            .contains("is computed when the KPAR is built")
    );
}

#[test]
fn no_index_symbols_is_refused_on_a_toml_project() {
    let mut project = toml_project();

    let error = do_include(
        &mut project,
        [Utf8UnixPathBuf::from("Parts.sysml")].into_iter(),
        false,
        false,
        None,
    )
    .expect_err("a computed field");

    assert!(matches!(error, IncludeError::ComputedField("index")));
}

#[test]
fn force_format_is_refused_on_a_toml_project() {
    let mut project = toml_project();

    let error = do_include(
        &mut project,
        [Utf8UnixPathBuf::from("Parts.sysml")].into_iter(),
        false,
        true,
        Some(crate::symbols::Language::SysML),
    )
    .expect_err("nothing to extract");

    assert!(matches!(error, IncludeError::ForceFormat));
}

#[test]
fn including_into_a_manifest_without_sources_says_to_add_one() {
    let bare = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\n";
    let mut project = InMemoryProject {
        model_toml: Some(bare.to_owned()),
        ..InMemoryProject::default()
    };

    // The manifest does not read -- `sources` is required -- so the
    // refusal comes from the validating read rather than from the edit.
    let error = include(&mut project, "Parts.sysml").expect_err("no sources");
    assert!(error.to_string().contains("sources"));
}

#[test]
fn excluding_a_source_removes_it_from_the_manifest() {
    let mut project = toml_project();
    include(&mut project, "Parts.sysml").expect("a new source");

    do_exclude(&mut project, ["Base.kerml"].into_iter()).expect("a listed source");

    assert_eq!(sources_of(&project), "sources = [\n    \"Parts.sysml\",\n]");
}

#[test]
fn excluding_keeps_the_comment_on_a_neighbouring_entry() {
    let commented = "manifest-version = 1\n\n\
                     [project]\n\
                     publisher = \"acme\"\n\
                     name = \"widgets\"\n\
                     version = \"1.0.0\"\n\
                     sources = [\n    \"Base.kerml\",  # the base\n    \"Parts.sysml\",\n]\n";
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&commented.parse().expect("a manifest"), false)
        .expect("stores a manifest");
    project
        .files
        .insert("Base.kerml".into(), "package Base;\n".to_owned());

    do_exclude(&mut project, ["Parts.sysml"].into_iter()).expect("a listed source");

    // A TOML comment binds forward, so `# the base` is stored at the
    // front of `Parts.sysml`'s prefix even though it is written on
    // Base's line. Removing Parts must not take it.
    assert_eq!(
        sources_of(&project),
        "sources = [\n    \"Base.kerml\",  # the base\n]"
    );
}

#[test]
fn excluding_a_path_that_is_not_listed_is_refused() {
    let mut project = toml_project();

    let error = do_exclude(&mut project, ["Parts.sysml"].into_iter()).expect_err("not listed");
    assert!(matches!(error, ExcludeError::SourceNotFound(_)));
}

#[test]
fn a_both_project_is_refused_by_either_command() {
    // Every field is public, so this is a state a caller can build.
    let mut project = toml_project();
    project.info = Some(crate::model::InterchangeProjectInfoRaw {
        name: "widgets".to_owned(),
        publisher: Some("acme".to_owned()),
        description: None,
        version: "1.0.0".to_owned(),
        license: None,
        maintainer: vec![],
        topic: vec![],
        usage: vec![],
        website: None,
    });

    assert!(include(&mut project, "Parts.sysml").is_err());
    assert!(do_exclude(&mut project, ["Base.kerml"].into_iter()).is_err());
}

#[test]
fn a_json_project_hashes_and_indexes() {
    let mut project = InMemoryProject::default();
    project
        .put_info(
            &crate::model::InterchangeProjectInfoRaw {
                name: "widgets".to_owned(),
                publisher: Some("acme".to_owned()),
                description: None,
                version: "1.0.0".to_owned(),
                license: None,
                maintainer: vec![],
                topic: vec![],
                usage: vec![],
                website: None,
            },
            false,
        )
        .expect("a json project");
    project
        .files
        .insert("Parts.sysml".into(), "package Parts;\n".to_owned());

    do_include(
        &mut project,
        [Utf8UnixPathBuf::from("Parts.sysml")].into_iter(),
        true,
        true,
        None,
    )
    .expect("a json project hashes and indexes");

    let meta = project.meta.as_ref().expect("metadata");
    assert!(meta.index.contains_key("Parts"));
    assert!(
        meta.checksum
            .as_ref()
            .expect("checksums")
            .contains_key("Parts.sysml")
    );
}
