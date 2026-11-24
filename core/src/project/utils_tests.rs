// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use typed_path::Utf8UnixPathBuf;

use crate::{
    model::{InterchangeProjectUsage, InterchangeProjectUsageRaw},
    project::utils::Identifier,
};

// --- Identifier constructors ---

#[test]
fn identifier_from_spelling_purl_safe() {
    assert_eq!(
        Identifier::from_spelling("acme-corp", "my-lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_spelling_publisher_normalized() {
    // Uppercase + space in publisher → lowercased, spaces become dashes
    assert_eq!(
        Identifier::from_spelling("ACME Corp", "my-lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_spelling_name_normalized() {
    // Uppercase + dot in name → lowercased
    assert_eq!(
        Identifier::from_spelling("acme-corp", "My.Lib").as_str(),
        "pkg:sysand/acme-corp/my.lib"
    );
}

#[test]
fn identifier_from_spelling_both_normalized() {
    // Both publisher and name need normalization
    assert_eq!(
        Identifier::from_spelling("ACME Corp", "My Lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_spelling_arbitrary_publisher_gives_urn() {
    // Publisher too short (< 3 chars) → falls through to urn:sysand: form
    assert_eq!(
        Identifier::from_spelling("ab", "my-lib").as_str(),
        "urn:sysand:ab/my-lib"
    );
}

#[test]
fn identifier_from_spelling_arbitrary_name_gives_urn() {
    // Name too short (< 3 chars) → falls through to urn:sysand: form
    assert_eq!(
        Identifier::from_spelling("acme-corp", "ab").as_str(),
        "urn:sysand:acme-corp/ab"
    );
}

#[test]
fn identifier_from_resource_usage_returns_iri_as_is() {
    let resource = Iri::parse("urn:kpar:test".to_owned()).unwrap();
    let usage = InterchangeProjectUsage::Resource {
        resource,
        version_constraint: None,
    };
    assert_eq!(Identifier::from(usage).as_str(), "urn:kpar:test");
}

#[test]
fn identifier_from_directory_usage_purl_safe() {
    let usage = InterchangeProjectUsage::Directory {
        dir: Utf8UnixPathBuf::from("dep"),
        publisher: crate::model::ProjectPublisher::parse("acme-corp".to_owned()).unwrap(),
        name: crate::model::ProjectName::parse("my-lib".to_owned()).unwrap(),
    };
    assert_eq!(
        Identifier::from(usage).as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_directory_usage_arbitrary_publisher_gives_urn() {
    // Short publisher → Arbitrary form → urn:sysand: (non-PURL, non-URL)
    let usage = InterchangeProjectUsage::Directory {
        dir: Utf8UnixPathBuf::from("dep"),
        publisher: crate::model::ProjectPublisher::parse("ab".to_owned()).unwrap(),
        name: crate::model::ProjectName::parse("my-lib".to_owned()).unwrap(),
    };
    assert_eq!(Identifier::from(usage).as_str(), "urn:sysand:ab/my-lib");
}

#[test]
fn identifier_from_kpar_path_usage_purl_safe() {
    let usage = InterchangeProjectUsage::KparPath {
        kpar_path: Utf8UnixPathBuf::from("dep.kpar"),
        publisher: crate::model::ProjectPublisher::parse("acme-corp".to_owned()).unwrap(),
        name: crate::model::ProjectName::parse("my-lib".to_owned()).unwrap(),
    };
    assert_eq!(
        Identifier::from(usage).as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_interchange_usage_unchecked_resource() {
    let usage = InterchangeProjectUsageRaw::Resource {
        resource: "urn:kpar:test".to_owned(),
        version_constraint: None,
    };
    assert_eq!(
        Identifier::from_interchange_usage_unchecked(&usage).as_str(),
        "urn:kpar:test"
    );
}

#[test]
fn identifier_from_interchange_usage_unchecked_directory() {
    let usage = InterchangeProjectUsageRaw::Directory {
        dir: "dep".to_owned(),
        publisher: "acme-corp".to_owned(),
        name: "my-lib".to_owned(),
    };
    assert_eq!(
        Identifier::from_interchange_usage_unchecked(&usage).as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_iri_unchecked_str() {
    assert_eq!(
        Identifier::from_iri_unchecked_str("urn:kpar:test").as_str(),
        "urn:kpar:test"
    );
}

#[test]
fn identifier_from_iri_owned() {
    let iri = Iri::parse("urn:kpar:test".to_owned()).unwrap();
    assert_eq!(Identifier::from_iri_owned(iri).as_str(), "urn:kpar:test");
}

/// Whether a project spelling gets a `pkg:sysand` identifier is decided by
/// the spelling as given, not by its normalized form: `ACME Inc./Foo`
/// normalizes to the index spelling `acme-inc/foo`, but is a different
/// project
#[test]
fn identifier_from_project_decides_by_the_spelling_as_given() {
    use crate::model::{IndexName, IndexPublisher, ProjectName, ProjectPublisher};

    let project = Identifier::from_project(
        &ProjectPublisher::parse("ACME Inc.".to_owned()).unwrap(),
        &ProjectName::parse("Foo".to_owned()).unwrap(),
    );
    assert_eq!(project.as_str(), "urn:sysand:acme-inc/foo");

    let index = Identifier::from_index(
        &IndexPublisher::parse("acme-inc".to_owned()).unwrap(),
        &IndexName::parse("foo".to_owned()).unwrap(),
    );
    assert_eq!(index.as_str(), "pkg:sysand/acme-inc/foo");
    assert_ne!(project, index);

    // An index spelling given as a project spelling is the same project
    let project = Identifier::from_project(
        &ProjectPublisher::parse("ACME Inc".to_owned()).unwrap(),
        &ProjectName::parse("Foo".to_owned()).unwrap(),
    );
    assert_eq!(project, index);
}

// --- write_atomic ---

#[cfg(all(feature = "filesystem", unix))]
#[test]
fn write_atomic_writes_through_a_symlink() {
    use std::os::unix::fs::symlink;

    use crate::project::utils::wrapfs;

    let dir = camino_tempfile::tempdir().unwrap();
    let target = dir.path().join("shared.json");
    let link = dir.path().join(".project.json");
    std::fs::write(&target, "old").unwrap();
    symlink(&target, &link).unwrap();

    wrapfs::write_atomic(&link, "new").unwrap();

    assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
}

#[cfg(all(feature = "filesystem", unix))]
#[test]
fn write_atomic_keeps_the_permissions_of_the_file_it_replaces() {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::project::utils::wrapfs;

    let dir = camino_tempfile::tempdir().unwrap();
    let path = dir.path().join("model.toml");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    wrapfs::write_atomic(&path, "new").unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
}

#[cfg(all(feature = "filesystem", unix))]
#[test]
fn write_atomic_leaves_no_sibling_behind_when_the_rename_fails() {
    use crate::project::utils::wrapfs;

    let dir = camino_tempfile::tempdir().unwrap();
    // A non-empty directory cannot be renamed over by a file.
    let path = dir.path().join("model.toml");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("inside"), "x").unwrap();

    wrapfs::write_atomic(&path, "new").unwrap_err();

    assert!(!wrapfs::exists(dir.path().join(".model.toml.sysand-tmp")).unwrap());
}
