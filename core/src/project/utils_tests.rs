// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::error::Error;

use camino::Utf8Path;
use fluent_uri::Iri;
use typed_path::Utf8UnixPathBuf;

use crate::{
    model::{InterchangeProjectUsage, InterchangeProjectUsageRaw},
    project::utils::{ExpectedIdentity, Identifier, IdentityMismatch, relativize_path},
    resolve::UsageOrigin,
};

#[test]
fn simple_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\")
    } else {
        Utf8Path::new("/")
    };
    assert_eq!(relativize_path(path, root)?, "a/b/c");
    Ok(())
}

#[test]
fn backtracking_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\d\e\f")
    } else {
        Utf8Path::new("/d/e/f")
    };
    assert_eq!(relativize_path(path, root)?, "../../../a/b/c");
    Ok(())
}

#[test]
fn trivial_relativize_path() -> Result<(), Box<dyn Error>> {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let relative = Utf8Path::new(".");
    assert_eq!(relativize_path(path, root)?, relative.as_str());
    Ok(())
}

#[test]
#[should_panic]
fn relativize_path_error_relative_path() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"a\b\c")
    } else {
        Utf8Path::new("a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_relative_root() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"a\b\c")
    } else {
        Utf8Path::new("a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_non_canonical() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\..\c")
    } else {
        Utf8Path::new("/a/../c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let _unreachable = relativize_path(path, root);
}

#[test]
#[should_panic]
fn relativize_path_error_non_canonical_root() {
    let path = if cfg!(windows) {
        Utf8Path::new(r"C:\a\b\c")
    } else {
        Utf8Path::new("/a/b/c")
    };
    let root = if cfg!(windows) {
        Utf8Path::new(r"C:\a\..\c")
    } else {
        Utf8Path::new("/a/../c")
    };
    let _unreachable = relativize_path(path, root);
}

#[cfg(target_os = "windows")]
#[test]
fn relativize_path_error_non_common_prefix() -> Result<(), Box<dyn Error>> {
    use crate::project::utils::RelativizePathError;

    let path = Utf8Path::new(r"C:\a\b\c");
    let root = Utf8Path::new(r"D:\a\b\c");
    let Err(err) = relativize_path(path, root) else {
        panic!("`relativize_path` did not return error");
    };
    let RelativizePathError::NoCommonPrefix {
        path: err_path,
        root: err_root,
    } = err;
    assert_eq!(*err_path, *path);
    assert_eq!(*err_root, *root);
    Ok(())
}

// --- Identifier constructors ---

#[test]
fn identifier_from_pub_name_purl_safe() {
    assert_eq!(
        Identifier::from_pub_name("acme-corp", "my-lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_pub_name_publisher_normalized() {
    // Uppercase + space in publisher → lowercased, spaces become dashes
    assert_eq!(
        Identifier::from_pub_name("ACME Corp", "my-lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_pub_name_name_normalized() {
    // Uppercase + dot in name → lowercased
    assert_eq!(
        Identifier::from_pub_name("acme-corp", "My.Lib").as_str(),
        "pkg:sysand/acme-corp/my.lib"
    );
}

#[test]
fn identifier_from_pub_name_both_normalized() {
    // Both publisher and name need normalization
    assert_eq!(
        Identifier::from_pub_name("ACME Corp", "My Lib").as_str(),
        "pkg:sysand/acme-corp/my-lib"
    );
}

#[test]
fn identifier_from_pub_name_arbitrary_publisher_gives_urn() {
    // Publisher too short (< 3 chars) → falls through to urn:sysand: form
    assert_eq!(
        Identifier::from_pub_name("ab", "my-lib").as_str(),
        "urn:sysand:ab/my-lib"
    );
}

#[test]
fn identifier_from_pub_name_arbitrary_name_gives_urn() {
    // Name too short (< 3 chars) → falls through to urn:sysand: form
    assert_eq!(
        Identifier::from_pub_name("acme-corp", "ab").as_str(),
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
/// the spelling as given, not by its normalized form (see
/// [`Identifier::from_project`]): `ACME Inc./Foo` normalizes to the index
/// spelling `acme-inc/foo`, but is a different project
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

/// model-toml.md "Normalization": "All projects are resolved by their
/// *normalized* identity, regardless of how the usage is declared."
/// Both directions, because neither side of the comparison is the
/// canonical one.
#[test]
fn an_identity_matches_after_normalization() {
    ExpectedIdentity::resolving(Some("Acme Corp".to_owned()), "widgets".to_owned())
        .check(Some("acme-corp"), "widgets")
        .expect("`Acme Corp` and `acme-corp` are one publisher");
    ExpectedIdentity::resolving(Some("acme-corp".to_owned()), "widgets".to_owned())
        .check(Some("Acme Corp"), "widgets")
        .expect("and the other way round");
}

/// "Two identities are equal when their publishers *and names* are equal
/// after normalization." Normalizing one and not the other would refuse
/// `My Lib` at a target named `my-lib` while accepting `Acme Corp` at
/// `acme-corp`.
#[test]
fn a_name_is_normalized_too() {
    ExpectedIdentity::resolving(Some("acme".to_owned()), "My Lib".to_owned())
        .check(Some("acme"), "my-lib")
        .expect("`My Lib` and `my-lib` are one name");
}

/// Normalization is not laxity: a different identity is still a
/// mismatch, and says which half of the pair differed.
#[test]
fn a_different_identity_is_still_a_mismatch() {
    assert_matches!(
        ExpectedIdentity::resolving(Some("acme".to_owned()), "widgets".to_owned())
            .check(Some("globex"), "widgets")
            .expect_err("another publisher"),
        IdentityMismatch::Publisher { expected, actual }
            if expected.as_deref() == Some("acme") && actual.as_deref() == Some("globex")
    );
    assert_matches!(
        ExpectedIdentity::resolving(Some("acme".to_owned()), "widgets".to_owned())
            .check(Some("acme"), "gadgets")
            .expect_err("another name"),
        IdentityMismatch::Name { expected, actual }
            if expected == "widgets" && actual == "gadgets"
    );
}

/// "Resolution is the only place that normalizes ... Every other
/// comparison is literal string equality." `sysand sync` checks an
/// installed project against the pair the lockfile recorded, where
/// normalizing would hide the drift the check exists to catch.
#[test]
fn a_recorded_identity_is_compared_literally() {
    assert_matches!(
        ExpectedIdentity::recorded(Some("Acme Corp".to_owned()), "widgets".to_owned())
            .check(Some("acme-corp"), "widgets")
            .expect_err("the lockfile recorded the other spelling"),
        IdentityMismatch::Publisher { .. }
    );
}

/// The manifest the usage was written in decides only what a
/// publisher-less target is told, and never turns a mismatch into a
/// match.
#[test]
fn only_a_toml_usage_gets_the_message_that_names_the_fix() {
    let expected = ExpectedIdentity::resolving(Some("acme".to_owned()), "widgets".to_owned());
    assert_matches!(
        expected
            .clone()
            .declared_in(UsageOrigin::Toml)
            .check(None, "widgets")
            .expect_err("the target declares no publisher"),
        IdentityMismatch::NoPublisher { name } if name == "widgets"
    );
    assert_matches!(
        expected
            .declared_in(UsageOrigin::Json)
            .check(None, "widgets")
            .expect_err("the same target, under a json usage"),
        IdentityMismatch::Publisher { expected, actual }
            if expected.as_deref() == Some("acme") && actual.is_none()
    );
}

/// A usage that expects a target with no publisher is not the same
/// thing as a target that is missing one: `expects_no_publisher_but_
/// project_has_one` pins the direction that stays a mismatch, and this
/// pins the direction that is a match.
#[test]
fn a_usage_may_expect_a_target_with_no_publisher() {
    ExpectedIdentity::resolving(None, "widgets".to_owned())
        .declared_in(UsageOrigin::Toml)
        .check(None, "widgets")
        .expect("neither side has a publisher");
    assert_matches!(
        ExpectedIdentity::resolving(None, "widgets".to_owned())
            .declared_in(UsageOrigin::Toml)
            .check(Some("surprise"), "widgets")
            .expect_err("the target has one after all"),
        IdentityMismatch::Publisher { expected, actual }
            if expected.is_none() && actual.as_deref() == Some("surprise")
    );
}
