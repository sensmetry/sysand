// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use super::{InMemoryError, InMemoryProject};
use crate::manifest::edit::DocumentMut;
use crate::model::InterchangeProjectInfoRaw;
use crate::project::{ProjectFormat, ProjectMut as _, ProjectRead as _};

const MANIFEST: &str = "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"widgets\"\nversion = \"1.0.0\"\nsources = []\n";

fn document(text: &str) -> DocumentMut {
    text.parse().expect("a valid manifest document")
}

fn info() -> InterchangeProjectInfoRaw {
    InterchangeProjectInfoRaw {
        name: "widgets".to_owned(),
        publisher: Some("acme".to_owned()),
        description: None,
        version: "1.0.0".to_owned(),
        license: None,
        maintainer: vec![],
        topic: vec![],
        usage: vec![],
        website: None,
    }
}

#[test]
fn a_memory_project_stores_and_reports_a_manifest() {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&document(MANIFEST), false)
        .expect("an empty project takes a manifest");

    assert_eq!(project.format().expect("a format"), ProjectFormat::Toml);
    assert_eq!(project.model_toml.as_deref(), Some(MANIFEST));
}

#[test]
fn a_memory_project_with_a_manifest_synthesizes_the_kpar_view() {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&document(MANIFEST), false)
        .expect("a manifest");

    // What a json-shaped caller is entitled to see, built from the
    // manifest rather than from `info`/`meta`, which are empty.
    let (info, meta) = project.get_project().expect("a view");
    let info = info.expect("synthesized info");
    assert_eq!(info.name, "widgets");
    assert_eq!(info.publisher.as_deref(), Some("acme"));
    assert!(meta.is_some());
}

#[test]
fn a_memory_project_with_a_manifest_refuses_a_json_write() {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&document(MANIFEST), false)
        .expect("a manifest");

    let error = project
        .put_info(&info(), false)
        .expect_err("a toml project takes no json documents");
    assert!(matches!(error, InMemoryError::JsonWriteToTomlProject(_)));
}

#[test]
fn a_memory_project_holding_both_is_ambiguous() {
    // Every field is public, so this is a state a caller can build. It
    // is answered rather than declared impossible.
    let project = InMemoryProject {
        info: Some(info()),
        model_toml: Some(MANIFEST.to_owned()),
        ..InMemoryProject::default()
    };

    assert_eq!(project.format().expect("a format"), ProjectFormat::Both);
    assert!(matches!(
        project.get_project(),
        Err(InMemoryError::AmbiguousFormat)
    ));
}

#[test]
fn writing_a_memory_manifest_that_is_there_is_refused_without_overwrite() {
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(&document(MANIFEST), false)
        .expect("a manifest");

    assert!(
        project
            .put_manifest_document(&document(MANIFEST), false)
            .is_err()
    );
    project
        .put_manifest_document(&document(&MANIFEST.replace("1.0.0", "2.0.0")), true)
        .expect("overwrite replaces it");
    assert!(
        project
            .model_toml
            .as_deref()
            .expect("a manifest")
            .contains("2.0.0")
    );
}

#[test]
fn a_memory_manifest_naming_a_missing_source_fails_the_view() {
    // Only a toml project can fail this way: its view reads the files
    // its `sources` name.
    let mut project = InMemoryProject::default();
    project
        .put_manifest_document(
            &document(&MANIFEST.replace("sources = []", "sources = [\"A.kerml\"]")),
            false,
        )
        .expect("a manifest");

    assert!(matches!(
        project.get_project(),
        Err(InMemoryError::FileNotFound(_))
    ));
}

#[test]
fn a_json_memory_project_refuses_a_manifest_write() {
    let mut project = InMemoryProject::default();
    project.put_info(&info(), false).expect("a json project");

    assert!(matches!(
        project.put_manifest_document(&document(MANIFEST), false),
        Err(crate::project::ManifestWriteError::Project(
            InMemoryError::TomlWriteToJsonProject
        ))
    ));
}
