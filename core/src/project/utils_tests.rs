// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::error::Error;

use camino::Utf8Path;
use fluent_uri::Iri;
use typed_path::Utf8UnixPathBuf;

use crate::{
    model::{InterchangeProjectUsage, InterchangeProjectUsageRaw},
    project::utils::{Identifier, relativize_path},
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
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
    let _ = relativize_path(path, root);
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
        publisher: "acme-corp".to_owned(),
        name: "my-lib".to_owned(),
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
        publisher: "ab".to_owned(),
        name: "my-lib".to_owned(),
    };
    assert_eq!(Identifier::from(usage).as_str(), "urn:sysand:ab/my-lib");
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
