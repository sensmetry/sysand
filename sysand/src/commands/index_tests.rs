// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{fs, io::Write as _};

use camino::Utf8Path;
use camino_tempfile::tempdir;
use serde_json::json;
use sysand_core::commands::index::INDEX_FILE_NAME;
use zip::write::SimpleFileOptions;

use super::{
    RemoveTarget, command_index_add, command_index_init, command_index_remove, command_index_yank,
};

// The tests only care about whether a fix was offered (`suggestion.is_some()`),
// not the exact wording, so that the messages can be edited freely.

const SOME_IRI: &str = "https://example.test/some-package";

fn corrupt_index_at(index_root: &Utf8Path) {
    fs::create_dir_all(index_root).unwrap();
    fs::write(index_root.join(INDEX_FILE_NAME), "not valid json").unwrap();
}

/// Writes a minimal KPAR with just enough of `.project.json`/`.meta.json`
/// for `do_index_add` to reach the publisher/name/iri checks.
fn write_kpar(kpar_path: &Utf8Path, publisher: &str, name: &str) {
    let info = json!({"name": name, "publisher": publisher, "version": "1.0.0"});
    let meta = json!({"index": {}, "created": "0000-01-01T00:00:00Z"});

    let file = fs::File::create(kpar_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    zip.start_file(".project.json", options).unwrap();
    zip.write_all(info.to_string().as_bytes()).unwrap();
    zip.start_file(".meta.json", options).unwrap();
    zip.write_all(meta.to_string().as_bytes()).unwrap();
    zip.finish().unwrap();
}

/// The suggestion offered for a fixable error, expected to spell out the
/// publisher/name rules (3 to 50 ASCII characters among them)
fn assert_suggests_the_naming_rules<T: sysand_core::env::utils::ErrorBound>(
    err: super::IndexError<T>,
) {
    let suggestion = err.suggestion.expect("expected a suggestion to be offered");
    assert!(suggestion.contains('3'), "suggestion was: {suggestion}");
    assert!(suggestion.contains("50"), "suggestion was: {suggestion}");
}

#[test]
fn init_on_an_existing_index_offers_no_fix() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();

    let err = command_index_init(tmp.path()).unwrap_err();
    assert!(err.suggestion.is_none());
}

#[test]
fn add_with_missing_index_root_offers_a_fix() {
    let tmp = tempdir().unwrap();
    let missing_root = tmp.path().join("does-not-exist");
    let kpar_path = tmp.path().join("whatever.kpar");

    let err = command_index_add::<&str, _, _>(None, kpar_path, &missing_root).unwrap_err();
    assert!(err.suggestion.is_some());
}

#[test]
fn add_with_corrupt_index_offers_a_fix() {
    let tmp = tempdir().unwrap();
    corrupt_index_at(tmp.path());
    let kpar_path = tmp.path().join("whatever.kpar");

    let err = command_index_add::<&str, _, _>(None, kpar_path, tmp.path()).unwrap_err();
    assert!(err.suggestion.is_some());
}

// Unlike `add`, a missing index root on `yank`/`remove` more likely means a
// wrong path or the wrong command than a not-yet-created index, so no fix
// is suggested there.
#[test]
fn yank_with_missing_index_root_offers_no_fix() {
    let tmp = tempdir().unwrap();
    let missing_root = tmp.path().join("does-not-exist");

    let err = command_index_yank(SOME_IRI, "1.0.0", &missing_root).unwrap_err();
    assert!(err.suggestion.is_none());
}

#[test]
fn yank_with_corrupt_index_offers_a_fix() {
    let tmp = tempdir().unwrap();
    corrupt_index_at(tmp.path());

    let err = command_index_yank(SOME_IRI, "1.0.0", tmp.path()).unwrap_err();
    assert!(err.suggestion.is_some());
}

#[test]
fn yank_of_unknown_project_offers_no_fix() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();

    let err = command_index_yank(SOME_IRI, "1.0.0", tmp.path()).unwrap_err();
    assert!(err.suggestion.is_none());
}

#[test]
fn remove_with_missing_index_root_offers_no_fix() {
    let tmp = tempdir().unwrap();
    let missing_root = tmp.path().join("does-not-exist");

    let err = command_index_remove(SOME_IRI, RemoveTarget::Project, &missing_root).unwrap_err();
    assert!(err.suggestion.is_none());
}

#[test]
fn remove_with_corrupt_index_offers_a_fix() {
    let tmp = tempdir().unwrap();
    corrupt_index_at(tmp.path());

    let err = command_index_remove(SOME_IRI, RemoveTarget::Project, tmp.path()).unwrap_err();
    assert!(err.suggestion.is_some());
}

#[test]
fn remove_of_unknown_project_offers_no_fix() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();

    let err = command_index_remove(SOME_IRI, RemoveTarget::Project, tmp.path()).unwrap_err();
    assert!(err.suggestion.is_none());
}

#[test]
fn add_of_a_too_short_publisher_mentions_the_naming_rules() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();
    let kpar_path = tmp.path().join("project.kpar");
    write_kpar(&kpar_path, "ab", "valid-name");

    let err = command_index_add::<&str, _, _>(None, &kpar_path, tmp.path()).unwrap_err();
    assert_suggests_the_naming_rules(err);
}

#[test]
fn add_of_a_too_short_name_mentions_the_naming_rules() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();
    let kpar_path = tmp.path().join("project.kpar");
    write_kpar(&kpar_path, "valid-publisher", "xy");

    let err = command_index_add::<&str, _, _>(None, &kpar_path, tmp.path()).unwrap_err();
    assert_suggests_the_naming_rules(err);
}

#[test]
fn add_with_iri_publisher_inconsistent_with_project_mentions_the_naming_rules() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();
    let kpar_path = tmp.path().join("project.kpar");
    write_kpar(&kpar_path, "the-publisher", "some-name");

    let err = command_index_add(
        Some("pkg:sysand/other-publisher/some-name"),
        &kpar_path,
        tmp.path(),
    )
    .unwrap_err();
    assert_suggests_the_naming_rules(err);
}

#[test]
fn add_with_iri_name_inconsistent_with_project_mentions_the_naming_rules() {
    let tmp = tempdir().unwrap();
    command_index_init(tmp.path()).unwrap();
    let kpar_path = tmp.path().join("project.kpar");
    write_kpar(&kpar_path, "the-publisher", "the-name");

    let err = command_index_add(
        Some("pkg:sysand/the-publisher/other-name"),
        &kpar_path,
        tmp.path(),
    )
    .unwrap_err();
    assert_suggests_the_naming_rules(err);
}
