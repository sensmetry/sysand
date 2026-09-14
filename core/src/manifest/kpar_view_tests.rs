// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use typed_path::Utf8UnixPathBuf;

use super::*;
use crate::model::{IndexName, IndexPublisher};

fn publisher(raw: &str) -> IndexPublisher {
    IndexPublisher::parse(raw).expect("a valid publisher")
}

fn name(raw: &str) -> IndexName {
    IndexName::parse(raw).expect("a valid name")
}

fn index(raw_publisher: &str, raw_name: &str, constraint: &str) -> Usage {
    Usage::Index {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        version_constraint: semver::VersionReq::parse(constraint).expect("a constraint"),
    }
}

fn dir(raw_publisher: &str, raw_name: &str, path: &str) -> Usage {
    Usage::SourcePath {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        dir: Utf8UnixPathBuf::from(path),
    }
}

fn kpar_path(raw_publisher: &str, raw_name: &str, path: &str) -> Usage {
    Usage::KparPath {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        kpar_path: Utf8UnixPathBuf::from(path),
    }
}

fn url(raw_publisher: &str, raw_name: &str, iri: &str) -> Usage {
    Usage::RemoteKpar {
        publisher: publisher(raw_publisher),
        name: name(raw_name),
        url: Iri::parse(iri.to_owned()).expect("a valid IRI"),
    }
}

/// model-toml.md "TOML to KPAR": "index: turn into a PURL
/// (`pkg:sysand/<publisher>/<name>`) with the version constraint as
/// declared".
#[test]
fn an_index_usage_becomes_a_purl_resource() {
    assert_eq!(
        usage(&index("acme", "widgets", "^1.2.3")),
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme/widgets".to_owned(),
            version_constraint: Some("^1.2.3".to_owned()),
        }
    );
}

/// A PURL is built from the *normalized* pair: model-toml.md
/// "Normalization", "All projects are resolved by their normalized
/// identity, regardless of how the usage is declared."
#[test]
fn a_purl_resource_is_built_from_the_normalized_pair() {
    assert_eq!(
        usage(&index("Acme Corp", "Wid Get", "^1")),
        InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme-corp/wid-get".to_owned(),
            version_constraint: Some("^1".to_owned()),
        }
    );
}

/// model-toml.md "TOML to KPAR": "remote-kpar: use the URL as-is"; the
/// kind takes no version constraint (Usages).
#[test]
fn a_url_usage_becomes_a_resource_with_no_constraint() {
    let target = "https://example.com/a/b.kpar?v=2#frag";
    assert_eq!(
        usage(&url("acme", "widgets", target)),
        InterchangeProjectUsageRaw::Resource {
            resource: target.to_owned(),
            version_constraint: None,
        }
    );
}

/// The view keeps a local usage typed, where the archive would rewrite
/// it into a PURL constrained to the target's version -- which needs the
/// target, and the resolver needs the path.
#[test]
fn a_dir_usage_becomes_a_typed_directory_usage() {
    assert_eq!(
        usage(&dir("acme", "widgets", "../lib")),
        InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "acme".to_owned(),
            name: "widgets".to_owned(),
        }
    );
}

#[test]
fn a_kpar_path_usage_becomes_a_typed_kpar_usage() {
    assert_eq!(
        usage(&kpar_path("acme", "widgets", "vendor/lib.kpar")),
        InterchangeProjectUsageRaw::KparPath {
            kpar_path: "vendor/lib.kpar".to_owned(),
            publisher: "acme".to_owned(),
            name: "widgets".to_owned(),
        }
    );
}

/// model-toml.md "TOML to KPAR": "A typed usage carries `publisher` and
/// `name` exactly as the manifest spells them, without normalization,
/// because Sysand requires them to match the values found at the
/// target." The opposite of the PURL above.
#[test]
fn a_typed_usage_keeps_the_pair_unnormalized() {
    assert_eq!(
        usage(&dir("Acme Corp", "Wid Get", "../lib")),
        InterchangeProjectUsageRaw::Directory {
            dir: "../lib".to_owned(),
            publisher: "Acme Corp".to_owned(),
            name: "Wid Get".to_owned(),
        }
    );
}

/// A view is consumed by the same code that reads a real KPAR, so every
/// usage it writes has to be one that code accepts. For the index kind
/// this is what pins the normalization above: `validate` refuses a
/// `pkg:sysand` IRI that is not in normalized form.
#[test]
fn a_view_usage_is_one_sysand_accepts() {
    for declared in [
        index("Acme Corp", "Wid Get", "^1.2.3"),
        url("acme", "widgets", "https://example.com/a/b.kpar"),
        dir("acme", "widgets", "../lib"),
        kpar_path("acme", "widgets", "vendor/lib.kpar"),
    ] {
        usage(&declared)
            .validate()
            .unwrap_or_else(|e| panic!("{declared:?} does not validate: {e}"));
    }
}

/// The identity of a usage is guaranteed encodable, but its *path* is
/// not checked at all when the manifest is read, unlike `sources`. So a
/// manifest the reader accepts can still produce a view that no consumer
/// validates -- with a message that names neither the key nor the file.
///
/// Pinned rather than fixed: the fix belongs to the manifest reader,
/// which would apply `parse_relative_unix_path` with the same two path
/// kinds `validate` uses here.
#[test]
fn a_usage_path_the_manifest_accepts_may_still_not_validate() {
    let error = usage(&dir("acme", "widgets", ""))
        .validate()
        .expect_err("an empty path");
    assert!(
        error.to_string().contains("references an invalid path"),
        "unexpected message: {error}"
    );
}
