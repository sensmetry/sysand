// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::{fmt::Display, num::NonZeroU64, slice, str::FromStr as _};

use crate::model::InterchangeProjectUsage;
use fluent_uri::Iri;

use toml_edit::DocumentMut;
use typed_path::Utf8UnixPathBuf;

use crate::utils::format_err;
use crate::{
    lock::{
        CURRENT_LOCK_VERSION, LOCKFILE_PREFIX, Lock, Project, Source, Usage, ValidationError,
        VersionError, check_lock_version,
    },
    project::ProjectChecksum,
};

#[test]
fn check_current_lock_version() {
    let version = CURRENT_LOCK_VERSION.to_owned();
    let document =
        DocumentMut::from_str(format!(r#"lock_version = "{version}""#).as_str()).unwrap();
    check_lock_version(&document).unwrap();
}

#[test]
fn check_unsupported_lock_version() {
    let version = "X";
    let document =
        DocumentMut::from_str(format!(r#"lock_version = "{version}""#).as_str()).unwrap();
    let Err(err) = check_lock_version(&document) else {
        panic!()
    };
    let VersionError::Unsupported(s) = &err else {
        panic!()
    };
    assert_eq!(s, version);
    assert_eq!(
        format_err(err),
        "lockfile version `X` is not supported; regenerate it with a lock operation"
    );
}

#[test]
fn old_registry_lockfile_is_rejected_by_version_gate() {
    let lockfile = format!(
        r#"{LOCKFILE_PREFIX}lock_version = "0.3"

[[project]]
name = "Old registry source"
version = "1.0.0"
sources = [{{ registry = "https://example.org" }}]
"#
    );

    let Err(err) = Lock::from_str(&lockfile) else {
        panic!()
    };
    let crate::lock::ParseError::Version(VersionError::Unsupported(s)) = &err else {
        panic!("expected unsupported version error, got {err:?}")
    };
    assert_eq!(s, "0.3");
    assert_eq!(
        format_err(err),
        "lockfile version `0.3` is not supported; regenerate it with a lock operation"
    );
}

#[test]
fn zero_index_kpar_size_is_rejected_by_lockfile_parse() {
    let lockfile = format!(
        r#"{LOCKFILE_PREFIX}lock_version = "{CURRENT_LOCK_VERSION}"

[[project]]
name = "Indexed"
version = "1.0.0"
sources = [{{ index_kpar = "https://example.org/project.kpar", kpar_size = 0, kpar_digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }}]
"#
    );

    let Err(err) = Lock::from_str(&lockfile) else {
        panic!()
    };
    let crate::lock::ParseError::Toml(_) = err else {
        panic!("expected TOML parse error for zero index kpar size, got {err:?}")
    };
}

#[test]
fn check_missing_lock_version() {
    let document = DocumentMut::from_str("").unwrap();
    let Err(err) = check_lock_version(&document) else {
        panic!()
    };
    let VersionError::Missing = err else { panic!() };
}

fn to_toml_matches_expected<D: Display>(projects: Vec<Project>, toml: D) {
    let lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects,
    };
    let expected = format!("{LOCKFILE_PREFIX}lock_version = \"{CURRENT_LOCK_VERSION}\"\n{toml}");
    assert_eq!(lock.to_string(), expected);
}

#[test]
fn minimal_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "a".to_owned(),
            publisher: None,
            version: "0.0.1".to_owned(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
name = "a"
version = "0.0.1"
"#,
    );
}

#[test]
fn many_projects_to_toml() {
    to_toml_matches_expected(
        vec![
            Project {
                name: "One".to_owned(),
                publisher: Some("Pub 1".to_owned()),
                version: "0.0.1".to_owned(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
            Project {
                name: "Two".to_owned(),
                publisher: None,
                version: "0.0.2".to_owned(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
            Project {
                name: "Three".to_owned(),
                publisher: Some("Pub 3".to_owned()),
                version: "0.0.3".to_owned(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
        ],
        r#"
[[project]]
publisher = "Pub 1"
name = "One"
version = "0.0.1"

[[project]]
name = "Two"
version = "0.0.2"

[[project]]
publisher = "Pub 3"
name = "Three"
version = "0.0.3"
"#,
    );
}

#[test]
fn one_export_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "One Package".to_owned(),
            publisher: None,
            version: "0.1.1".to_owned(),
            exports: vec!["PackageName".to_owned()],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
name = "One Package"
version = "0.1.1"
exports = [
    "PackageName",
]
"#,
    );
}

#[test]
fn many_exports_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "Three Packages".to_owned(),
            publisher: None,
            version: "0.1.3".to_owned(),
            exports: vec![
                "Package1".to_owned(),
                "Package2".to_owned(),
                "Package3".to_owned(),
            ],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
name = "Three Packages"
version = "0.1.3"
exports = [
    "Package1",
    "Package2",
    "Package3",
]
"#,
    );
}

#[test]
fn one_iri_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "One IRI".to_owned(),
            publisher: None,
            version: "0.2.1".to_owned(),
            exports: vec![],
            identifiers: vec!["urn:kpar:example".to_owned()],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
name = "One IRI"
version = "0.2.1"
identifiers = [
    "urn:kpar:example",
]
"#,
    );
}

#[test]
fn many_identifiers_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "Three IRI:s".to_owned(),
            publisher: None,
            version: "0.2.3".to_owned(),
            exports: vec![],
            identifiers: vec![
                "urn:kpar:example".to_owned(),
                "ftp://www.example.com".to_owned(),
                "http://www.example.com".to_owned(),
            ],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
name = "Three IRI:s"
version = "0.2.3"
identifiers = [
    "urn:kpar:example",
    "ftp://www.example.com",
    "http://www.example.com",
]
"#,
    );
}

#[test]
fn one_source_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "One source".to_owned(),
            publisher: None,
            version: "0.4.1".to_owned(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![Source::Editable {
                editable: Utf8UnixPathBuf::from("."),
            }],
        }],
        r#"
[[project]]
name = "One source"
version = "0.4.1"
sources = [
    { editable = "." },
]
"#,
    );
}

#[test]
fn many_sources_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "Seven sources".to_owned(),
            publisher: None,
            version: "0.4.7".to_owned(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![
                Source::Editable {
                    editable: Utf8UnixPathBuf::from("example/path"),
                },
                Source::LocalKpar {
                    kpar_path: Utf8UnixPathBuf::from("example.kpar"),
                    kpar_size: NonZeroU64::new(64).unwrap(),
                    kpar_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                },
                Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("example/path"),
                    checksum: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                },
                Source::RemoteKpar {
                    remote_kpar: "www.example.com/remote.kpar".to_owned(),
                    kpar_size: NonZeroU64::new(64).unwrap(),
                    kpar_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                },
                Source::IndexKpar {
                    index_kpar: "www.example.com/index.kpar".to_owned(),
                    kpar_size: NonZeroU64::new(128).unwrap(),
                    kpar_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                },
                Source::RemoteSrc {
                    remote_src: "www.example.com/remote".to_owned(),
                    checksum: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_owned(),
                },
                Source::RemoteGit {
                    remote_git: "github.com/example/remote.git".to_owned(),
                },
            ],
        }],
        r#"
[[project]]
name = "Seven sources"
version = "0.4.7"
sources = [
    { editable = "example/path" },
    { kpar_path = "example.kpar", kpar_size = 64, kpar_digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { src_path = "example/path", checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { remote_kpar = "www.example.com/remote.kpar", kpar_size = 64, kpar_digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { index_kpar = "www.example.com/index.kpar", kpar_size = 128, kpar_digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { remote_src = "www.example.com/remote", checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
    { remote_git = "github.com/example/remote.git" },
]
"#,
    );
}

#[test]
fn one_usage_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "One usage".to_owned(),
            publisher: None,
            version: "0.5.1".to_owned(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![Usage::from_str_unchecked("urn:kpar:usage")],
            sources: vec![],
        }],
        r#"
[[project]]
name = "One usage"
version = "0.5.1"
usages = [
    "urn:kpar:usage",
]
"#,
    );
}

#[test]
fn many_usage_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "Three usages".to_owned(),
            publisher: None,
            version: "0.5.3".to_owned(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![
                Usage::from_str_unchecked("urn:kpar:first"),
                Usage::from_str_unchecked("urn:kpar:second"),
                Usage::from_str_unchecked("urn:kpar:third"),
            ],
            sources: vec![],
        }],
        r#"
[[project]]
name = "Three usages"
version = "0.5.3"
usages = [
    "urn:kpar:first",
    "urn:kpar:second",
    "urn:kpar:third",
]
"#,
    );
}

fn roundtrip_makes_no_changes<D: Display>(toml: D) {
    let expected = format!("{LOCKFILE_PREFIX}lock_version = \"{CURRENT_LOCK_VERSION}\"\n{toml}");
    let lockfile: Lock = toml::from_str(&expected).unwrap();
    assert_eq!(lockfile.to_string(), expected);
}

#[test]
fn simple_roundtrip() {
    roundtrip_makes_no_changes(
        r#"
[[project]]
name = "Simple"
version = "0.0.1"
"#,
    );
}

#[test]
fn complex_roundtrip() {
    roundtrip_makes_no_changes(
        r#"
[[project]]
name = "One"
version = "0.0.1"
exports = [
    "Package1",
    "Package2",
    "Package3",
]
usages = [
    "urn:kpar:usage",
]

[[project]]
name = "Two"
version = "0.0.2"
exports = [
    "PackageName",
]
identifiers = [
    "urn:kpar:example",
    "ftp://www.example.com",
    "http://www.example.com",
]

[[project]]
name = "Three"
version = "0.0.3"
identifiers = [
    "urn:kpar:example",
]
usages = [
    "urn:kpar:first",
    "urn:kpar:second",
    "urn:kpar:third",
]
"#,
    );
}

fn make_project<N: AsRef<str>, S: AsRef<str>>(
    name: N,
    publisher: Option<String>,
    version: S,
    exports: &[&'static str],
    identifiers: &[&'static str],
    usages: &[Usage],
) -> Project {
    Project {
        name: name.as_ref().into(),
        publisher,
        version: version.as_ref().to_owned(),
        exports: exports.iter().map(|s| String::from(*s)).collect(),
        identifiers: identifiers.iter().map(|s| String::from(*s)).collect(),
        usages: usages.to_vec(),
        sources: vec![],
    }
}

#[test]
fn validate_empty() {
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_minimal() {
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![make_project("a", None, "0.0.1", &[], &[], &[])],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_single_usage() {
    let iri = "urn:kpar:test";
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage::from_str_unchecked(iri)],
            ),
            make_project("b", None, "0.0.1", &[], &[iri], &[]),
        ],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_multiple_usage() {
    let iri1 = "urn:kpar:test1";
    let iri2 = "urn:kpar:test2";
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[
                    Usage::from_str_unchecked(iri1),
                    Usage::from_str_unchecked(iri2),
                ],
            ),
            make_project("b", None, "0.0.1", &[], &[iri1], &[]),
            make_project("c", None, "0.0.1", &[], &[iri2], &[]),
        ],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_chained_usages() {
    let iri1 = "urn:kpar:test1";
    let iri2 = "urn:kpar:test2";
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage::from_str_unchecked(iri1)],
            ),
            make_project(
                "b",
                None,
                "0.0.1",
                &[],
                &[iri1],
                &[Usage::from_str_unchecked(iri2)],
            ),
            make_project("c", None, "0.0.1", &[], &[iri2], &[]),
        ],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_unsupported_lock_version() {
    let version = "X";
    let Err(err) = Lock {
        lock_version: version.to_owned(),
        projects: vec![],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::UnsupportedVersion(s) = &err else {
        panic!()
    };
    assert_eq!(s, version);
    assert_eq!(
        format_err(err),
        "lockfile version `X` is not supported; regenerate it with a lock operation"
    );
}

#[test]
fn validate_single_name_collision() {
    let name = "PackageName";
    let iri = "urn:kpar:test";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[name],
                &[],
                &[Usage::from_str_unchecked(iri)],
            ),
            make_project("b", None, "0.0.1", &[name], &[iri], &[]),
        ],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::NameCollision(s) = err else {
        panic!()
    };
    assert_eq!(s, name);
}

#[test]
fn validate_multiple_name_collision() {
    let name1 = "PackageName1";
    let name2 = "PackageName2";
    let name3 = "PackageName3";
    let name4 = "PackageName5";
    let iri = "urn:kpar:test";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[name1, name2, name3],
                &[],
                &[Usage::from_str_unchecked(iri)],
            ),
            make_project("b", None, "0.0.1", &[name2, name3, name4], &[iri], &[]),
        ],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::NameCollision(_) = err else {
        panic!()
    };
}

#[test]
fn validate_unsatisfied_usage() {
    let usage_in = Usage::from_str_unchecked("urn:kpar:test");
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![make_project(
            "a",
            None,
            "0.0.1",
            &[],
            &[],
            slice::from_ref(&usage_in),
        )],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::UnsatisfiedUsage { usage, name } = err else {
        panic!()
    };
    assert_eq!(usage, *usage_in);
    assert_eq!(name, "a");
}

#[test]
fn validate_checksum_invalid_digest_all_source_types() {
    // 64 chars but contains 'X' — right length, wrong character.
    const INVALID: &str = "dA8747a6f27A32f10Ba393113bCE29fX88181037a71f093f90e0ad5829D2b780";

    let cases: Vec<(&str, Source)> = vec![
        (
            "LocalSrc",
            Source::LocalSrc {
                src_path: Utf8UnixPathBuf::from("../path/to/the/project"),
                checksum: INVALID.to_owned(),
            },
        ),
        (
            "RemoteSrc",
            Source::RemoteSrc {
                remote_src: "https://example.com/src".to_owned(),
                checksum: INVALID.to_owned(),
            },
        ),
        (
            "LocalKpar",
            Source::LocalKpar {
                kpar_path: Utf8UnixPathBuf::from("project.kpar"),
                kpar_size: NonZeroU64::new(1).unwrap(),
                kpar_digest: INVALID.to_owned(),
            },
        ),
        (
            "RemoteKpar",
            Source::RemoteKpar {
                remote_kpar: "https://example.com/project.kpar".to_owned(),
                kpar_size: NonZeroU64::new(1).unwrap(),
                kpar_digest: INVALID.to_owned(),
            },
        ),
        (
            "IndexKpar",
            Source::IndexKpar {
                index_kpar: "https://example.com/indexed.kpar".to_owned(),
                kpar_size: NonZeroU64::new(1).unwrap(),
                kpar_digest: INVALID.to_owned(),
            },
        ),
    ];

    for (label, source) in cases {
        let Err(err) = Lock {
            lock_version: CURRENT_LOCK_VERSION.to_owned(),
            projects: vec![Project {
                name: "a".into(),
                publisher: None,
                version: "0.0.1".to_owned(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![source],
            }],
        }
        .validate() else {
            panic!("expected InvalidDigestFormat for {label}")
        };
        let ValidationError::InvalidDigestFormat { digest, name, kind } = err else {
            panic!("wrong error variant for {label}: {err:?}")
        };
        assert_eq!(digest, INVALID, "{label}");
        assert_eq!(name, "a", "{label}");
        assert_matches!(kind, "kpar" | "project canonical", "{label}");
    }
}

#[test]
fn validate_kpar_digest_rejects_uppercase() {
    let invalid_digest = "dA8747a6f27A32f10Ba393113bCe29f788181037a71f093f90e0ad5829d2b780";
    let err = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![Project {
            name: "Indexed".to_owned(),
            publisher: None,
            version: "0.0.1".to_owned(),
            exports: vec![],
            identifiers: vec!["urn:kpar:indexed".to_owned()],
            usages: vec![],
            sources: vec![Source::IndexKpar {
                index_kpar: "https://example.com/indexed.kpar".to_owned(),
                kpar_size: std::num::NonZeroU64::new(123).unwrap(),
                kpar_digest: invalid_digest.to_owned(),
            }],
        }],
    }
    .validate()
    .unwrap_err();
    let ValidationError::InvalidDigestFormat { digest, kind, name } = err else {
        panic!()
    };
    assert_eq!(digest, invalid_digest);
    assert_eq!(name, "Indexed");
    assert_eq!(kind, "kpar");
}

#[test]
fn sort_empty() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![]);
}

#[test]
fn sort_single_trivial() {
    let project = make_project("a", None, "0.0.1", &[], &[], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project]);
}

#[test]
fn sort_exports() {
    let project1 = make_project("a", None, "0.0.1", &["B", "A"], &[], &[]);
    let project2 = make_project("a", None, "0.0.1", &["A", "B"], &[], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

// `identifiers` must not be sorted by `Lock::sort()`
// (unlike `exports`, `usages` and `sources`, which are). Other projects'
// `usages` entries are assumed to reference a project's *first*
// identifier (see `validate_usages` and `remove_usage`), so
// reordering `identifiers` would desync that reference
#[test]
fn sort_does_not_reorder_identifiers() {
    let project = make_project("a", None, "0.0.1", &[], &["urn:kpar:b", "urn:kpar:a"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project]);
}

// End-to-end test for the same invariant as `sort_does_not_reorder_identifiers`
#[test]
fn canonicalize_preserves_first_identifier_used_by_usage() {
    let dep = make_project(
        "dep",
        None,
        "0.0.1",
        &[],
        // Deliberately not alphabetically sorted: "urn:kpar:z-alias" is the
        // one that `root`'s usage below refers to, and it must stay first.
        &["urn:kpar:z-alias", "urn:kpar:a-alias"],
        &[],
    );
    let root = make_project(
        "root",
        None,
        "0.0.1",
        &[],
        &["urn:kpar:root"],
        &[Usage::from_str_unchecked("urn:kpar:z-alias")],
    );
    let lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![dep, root],
    }
    .canonicalize();

    // `validate` checks that projects are referred to by their first identifier
    assert!(lock.validate().is_ok(), "{:?}", lock.validate());
}

#[test]
fn sort_sources() {
    let usage1 = Usage::from_str_unchecked("urn:kpar:a");
    let usage2 = Usage::from_str_unchecked("urn:kpar:b");
    let project1 = make_project(
        "a",
        None,
        "0.0.1",
        &[],
        &[],
        &[usage2.clone(), usage1.clone()],
    );
    let project2 = make_project("a", None, "0.0.1", &[], &[], &[usage1, usage2]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

#[test]
fn sort_sources_with_constraints() {
    let usage1 = Usage::from_str_unchecked("urn:kpar:a");
    let usage2 = Usage::from_str_unchecked("urn:kpar:a");
    let project1 = make_project(
        "a",
        None,
        "0.0.1",
        &[],
        &[],
        &[usage2.clone(), usage1.clone()],
    );
    let project2 = make_project("a", None, "0.0.1", &[], &[], &[usage1, usage2]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

#[test]
fn sort_projects_by_name() {
    let project1 = make_project("A", None, "0.0.2", &["B"], &["urn:kpar:b"], &[]);
    let project2 = make_project("B", None, "0.0.1", &["A"], &["urn:kpar:a"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project2.clone(), project1.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project1, project2]);
}

#[test]
fn sort_projects_by_exports() {
    let project1 = make_project("A", None, "0.0.2", &["A"], &["urn:kpar:b"], &[]);
    let project2 = make_project("B", None, "0.0.1", &["B"], &["urn:kpar:a"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project2.clone(), project1.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project1, project2]);
}

#[test]
fn sort_projects_by_identifiers() {
    let project1 = make_project("A", None, "0.0.2", &["A"], &["urn:kpar:a"], &[]);
    let project2 = make_project("B", None, "0.0.1", &["A"], &["urn:kpar:b"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project2.clone(), project1.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project1, project2]);
}

#[test]
fn sort_projects_by_version() {
    let project1 = make_project("A", None, "0.0.1", &["A"], &["urn:kpar:a"], &[]);
    let project2 = make_project("B", None, "0.0.2", &["A"], &["urn:kpar:a"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![project2.clone(), project1.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project1, project2]);
}

#[test]
fn canonicalize_checksums() {
    const MIXED: &str = "dA8747a6f27A32f10Ba393113bCE29f788181037a71f093f90e0ad5829D2b780";
    const LOWER: &str = "da8747a6f27a32f10ba393113bce29f788181037a71f093f90e0ad5829d2b780";

    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![Project {
            name: "a".into(),
            publisher: None,
            version: "0.0.1".to_owned(),
            exports: vec![],
            identifiers: vec!["urn:kpar:a".to_owned()],
            usages: vec![],
            sources: vec![
                Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("../path/to/the/project"),
                    checksum: MIXED.to_owned(),
                },
                Source::RemoteSrc {
                    remote_src: "https://example.com/src".to_owned(),
                    checksum: MIXED.to_owned(),
                },
                Source::LocalKpar {
                    kpar_path: Utf8UnixPathBuf::from("project.kpar"),
                    kpar_size: NonZeroU64::new(1).unwrap(),
                    kpar_digest: MIXED.to_owned(),
                },
                Source::RemoteKpar {
                    remote_kpar: "https://example.com/project.kpar".to_owned(),
                    kpar_size: NonZeroU64::new(1).unwrap(),
                    kpar_digest: MIXED.to_owned(),
                },
                Source::IndexKpar {
                    index_kpar: "https://example.com/indexed.kpar".to_owned(),
                    kpar_size: NonZeroU64::new(1).unwrap(),
                    kpar_digest: MIXED.to_owned(),
                },
                Source::Editable {
                    editable: Utf8UnixPathBuf::from("editable/path"),
                },
                Source::RemoteGit {
                    remote_git: "https://github.com/example/example.git".to_owned(),
                },
            ],
        }],
    };
    lock.canonicalize_checksums();
    let Lock { projects, .. } = lock;
    let [project] = projects.as_slice() else {
        panic!()
    };

    let Source::LocalSrc { checksum, .. } = &project.sources[0] else {
        panic!()
    };
    assert_eq!(checksum, LOWER);

    let Source::RemoteSrc { checksum, .. } = &project.sources[1] else {
        panic!()
    };
    assert_eq!(checksum, LOWER);

    let Source::LocalKpar { kpar_digest, .. } = &project.sources[2] else {
        panic!()
    };
    assert_eq!(kpar_digest, LOWER);

    let Source::RemoteKpar { kpar_digest, .. } = &project.sources[3] else {
        panic!()
    };
    assert_eq!(kpar_digest, LOWER);

    let Source::IndexKpar { kpar_digest, .. } = &project.sources[4] else {
        panic!()
    };
    assert_eq!(kpar_digest, LOWER);

    // Editable and RemoteGit carry no checksum; their presence here confirms
    // canonicalize_checksums does not panic on them.
    assert_matches!(&project.sources[5], Source::Editable { .. });
    assert_matches!(&project.sources[6], Source::RemoteGit { .. });
}

// --- Lock identifiers ---

#[test]
fn usage_from_resource_usage_is_its_iri() {
    let iri = Iri::parse("urn:kpar:my-dep".to_owned()).unwrap();
    let interchange = InterchangeProjectUsage::Resource {
        resource: iri,
        version_constraint: None,
    };
    let usage = Usage::from(&interchange);
    assert_eq!(usage.inner(), "urn:kpar:my-dep");
}

#[test]
fn usage_from_directory_usage_is_sysand_purl() {
    let interchange = InterchangeProjectUsage::Directory {
        dir: Utf8UnixPathBuf::from("dep"),
        publisher: "acme-corp".to_owned(),
        name: "my-lib".to_owned(),
    };
    let usage = Usage::from(&interchange);
    assert_eq!(usage.inner(), "pkg:sysand/acme-corp/my-lib");
}

#[test]
fn usage_from_directory_usage_with_short_publisher_is_urn_sysand() {
    // Short publisher → Arbitrary form → urn:sysand: (non-PURL, non-URL)
    let interchange = InterchangeProjectUsage::Directory {
        dir: Utf8UnixPathBuf::from("dep"),
        publisher: "ab".to_owned(),
        name: "my-lib".to_owned(),
    };
    let usage = Usage::from(&interchange);
    assert_eq!(usage.inner(), "urn:sysand:ab/my-lib");
}

#[test]
fn lock_project_with_urn_sysand_identifier_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "my-lib".to_owned(),
            publisher: Some("ab".to_owned()),
            version: "1.0.0".to_owned(),
            exports: vec![],
            identifiers: vec!["urn:sysand:ab/my-lib".to_owned()],
            usages: vec![],
            sources: vec![],
        }],
        r#"
[[project]]
publisher = "ab"
name = "my-lib"
version = "1.0.0"
identifiers = [
    "urn:sysand:ab/my-lib",
]
"#,
    );
}

#[test]
fn lock_project_with_urn_sysand_identifier_roundtrip() {
    roundtrip_makes_no_changes(
        r#"
[[project]]
publisher = "ab"
name = "my-lib"
version = "1.0.0"
identifiers = [
    "urn:sysand:ab/my-lib",
]

[[project]]
name = "consumer"
version = "2.0.0"
usages = [
    "urn:sysand:ab/my-lib",
]
"#,
    );
}

#[test]
fn validate_usage_of_directory_derived_identifier() {
    // A project identified by urn:sysand: can be depended on via its identifier
    let dep_id = "urn:sysand:ab/my-lib";
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            make_project(
                "consumer",
                None,
                "2.0.0",
                &[],
                &[],
                &[Usage::from_str_unchecked(dep_id)],
            ),
            make_project(
                "my-lib",
                Some("ab".to_owned()),
                "1.0.0",
                &[],
                &[dep_id],
                &[],
            ),
        ],
    }
    .validate()
    .unwrap();
}

#[test]
fn old_lockfile_version_0_4_is_rejected() {
    let lockfile = format!(
        r#"{LOCKFILE_PREFIX}lock_version = "0.4"

[[project]]
name = "Old project"
version = "1.0.0"
"#
    );

    let Err(err) = Lock::from_str(&lockfile) else {
        panic!()
    };
    let crate::lock::ParseError::Version(VersionError::Unsupported(s)) = &err else {
        panic!("expected unsupported version error, got {err:?}")
    };
    assert_eq!(s, "0.4");
}

#[test]
fn source_to_checksum_editable_is_none() {
    let source = Source::Editable {
        editable: Utf8UnixPathBuf::from("."),
    };
    assert!(source.to_checksum().is_none());
}

#[test]
fn source_to_checksum_remote_git_is_none() {
    let source = Source::RemoteGit {
        remote_git: "https://github.com/example/example.git".to_owned(),
    };
    assert!(source.to_checksum().is_none());
}

#[test]
fn source_to_checksum_local_src_is_project_variant() {
    let checksum = "a".repeat(64);
    let source = Source::LocalSrc {
        src_path: Utf8UnixPathBuf::from("path/to/src"),
        checksum: checksum.clone(),
    };
    assert_eq!(
        source.to_checksum(),
        Some(ProjectChecksum::Project(checksum))
    );
}

#[test]
fn source_to_checksum_remote_src_is_project_variant() {
    let checksum = "b".repeat(64);
    let source = Source::RemoteSrc {
        remote_src: "https://example.com/src".to_owned(),
        checksum: checksum.clone(),
    };
    assert_eq!(
        source.to_checksum(),
        Some(ProjectChecksum::Project(checksum))
    );
}

#[test]
fn source_to_checksum_local_kpar_is_kpar_variant() {
    let digest = "c".repeat(64);
    let source = Source::LocalKpar {
        kpar_path: Utf8UnixPathBuf::from("project.kpar"),
        kpar_size: NonZeroU64::new(1).unwrap(),
        kpar_digest: digest.clone(),
    };
    assert_eq!(source.to_checksum(), Some(ProjectChecksum::Kpar(digest)));
}

#[test]
fn source_to_checksum_remote_kpar_is_kpar_variant() {
    let digest = "d".repeat(64);
    let source = Source::RemoteKpar {
        remote_kpar: "https://example.com/project.kpar".to_owned(),
        kpar_size: NonZeroU64::new(1).unwrap(),
        kpar_digest: digest.clone(),
    };
    assert_eq!(source.to_checksum(), Some(ProjectChecksum::Kpar(digest)));
}

#[test]
fn source_to_checksum_index_kpar_is_kpar_variant() {
    let digest = "e".repeat(64);
    let source = Source::IndexKpar {
        index_kpar: "https://example.com/indexed.kpar".to_owned(),
        kpar_size: NonZeroU64::new(1).unwrap(),
        kpar_digest: digest.clone(),
    };
    assert_eq!(source.to_checksum(), Some(ProjectChecksum::Kpar(digest)));
}

// ---- `Lock::is_root` and `Lock::remove_usage` ----

fn project_with_sources(sources: Vec<Source>) -> Project {
    Project {
        publisher: None,
        name: "p".to_owned(),
        version: "1.0.0".to_owned(),
        exports: vec![],
        identifiers: vec![],
        usages: vec![],
        sources,
    }
}

/// A root project (the project being operated on directly), as produced by
/// `do_lock_local_editable(".", ...)` for an ordinary, non-workspace project.
fn root_project(publisher: Option<&str>, name: &str, usages: &[&str]) -> Project {
    workspace_root_project(publisher, name, ".", usages)
}

/// A root project living at a workspace-relative subpath, as a workspace
/// member's own lock entry would be represented.
fn workspace_root_project(
    publisher: Option<&str>,
    name: &str,
    subpath: &str,
    usages: &[&str],
) -> Project {
    Project {
        publisher: publisher.map(str::to_owned),
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        exports: vec![],
        identifiers: vec![],
        usages: usages
            .iter()
            .map(|u| Usage::from_str_unchecked(u))
            .collect(),
        sources: vec![Source::Editable {
            editable: Utf8UnixPathBuf::from(subpath),
        }],
    }
}

/// A non-root (dependency) project, identified by `identifier`.
fn dep_project(name: &str, identifier: &str, usages: &[&str]) -> Project {
    Project {
        publisher: None,
        name: name.to_owned(),
        version: "1.0.0".to_owned(),
        exports: vec![],
        identifiers: vec![identifier.to_owned()],
        usages: usages
            .iter()
            .map(|u| Usage::from_str_unchecked(u))
            .collect(),
        sources: vec![Source::RemoteGit {
            remote_git: format!("https://example.com/{name}.git"),
        }],
    }
}

fn project_names(lock: &Lock) -> Vec<String> {
    let mut names: Vec<_> = lock.projects.iter().map(|p| p.name.clone()).collect();
    names.sort();
    names
}

#[test]
fn is_root_true_for_dot_editable() {
    // The nominal path used for the current project's own lock entry
    // (see `EditableProject`/`do_lock_local_editable(".", ...)`).
    let project = project_with_sources(vec![Source::Editable {
        editable: Utf8UnixPathBuf::from("."),
    }]);
    assert!(Lock::is_root(&project));
}

#[test]
fn is_root_true_for_subpath_editable() {
    // The nominal path used for a workspace member's lock entry.
    let project = project_with_sources(vec![Source::Editable {
        editable: Utf8UnixPathBuf::from("packages/foo"),
    }]);
    assert!(Lock::is_root(&project));
}

#[test]
fn is_root_false_for_parent_relative_editable() {
    let project = project_with_sources(vec![Source::Editable {
        editable: Utf8UnixPathBuf::from(".."),
    }]);
    assert!(!Lock::is_root(&project));
}

#[test]
fn is_root_false_for_non_editable_source() {
    let project = project_with_sources(vec![Source::RemoteGit {
        remote_git: "https://example.com/foo.git".to_owned(),
    }]);
    assert!(!Lock::is_root(&project));
}

#[test]
fn is_root_false_for_multiple_sources() {
    let project = project_with_sources(vec![
        Source::Editable {
            editable: Utf8UnixPathBuf::from("."),
        },
        Source::RemoteGit {
            remote_git: "https://example.com/foo.git".to_owned(),
        },
    ]);
    assert!(!Lock::is_root(&project));
}

#[test]
fn is_root_false_for_no_sources() {
    let project = project_with_sources(vec![]);
    assert!(!Lock::is_root(&project));
}

#[test]
fn remove_usage_prunes_unreachable_dependency() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            root_project(Some("me"), "root", &["urn:dep"]),
            dep_project("dep", "urn:dep", &[]),
        ],
    };

    let removed = lock
        .remove_usage(Some("me"), "root", "urn:dep")
        .expect("root project should be found");

    assert_eq!(project_names(&lock), vec!["root".to_owned()]);
    assert_eq!(lock.projects[0].usages, []);
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].name, "dep");
}

#[test]
fn remove_usage_returns_none_when_root_not_found() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            root_project(Some("me"), "root", &["urn:dep"]),
            dep_project("dep", "urn:dep", &[]),
        ],
    };

    let removed = lock.remove_usage(Some("someone-else"), "root", "urn:dep");

    assert_eq!(removed, None);
    assert_eq!(
        project_names(&lock),
        vec!["dep".to_owned(), "root".to_owned()]
    );
    assert_eq!(lock.projects[0].usages.len(), 1);
}

#[test]
fn remove_usage_returns_empty_when_usage_not_present() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            root_project(Some("me"), "root", &["urn:dep"]),
            dep_project("dep", "urn:dep", &[]),
        ],
    };

    let removed = lock.remove_usage(Some("me"), "root", "urn:other");

    assert_eq!(removed, Some(Vec::new()));
    assert_eq!(
        project_names(&lock),
        vec!["dep".to_owned(), "root".to_owned()]
    );
    assert_eq!(lock.projects[0].usages.len(), 1);
}

#[test]
fn remove_usage_keeps_dependency_needed_by_sibling_root() {
    // Two workspace members (`app`, `lib`) both use `shared`. Removing
    // `app`'s usage must not prune `shared`, since `lib` still needs it.
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            workspace_root_project(Some("me"), "app", "app", &["urn:shared"]),
            workspace_root_project(Some("me"), "lib", "lib", &["urn:shared"]),
            dep_project("shared", "urn:shared", &[]),
        ],
    };

    let removed = lock
        .remove_usage(Some("me"), "app", "urn:shared")
        .expect("root project should be found");

    assert!(removed.is_empty(), "removed = {removed:?}");
    assert_eq!(
        project_names(&lock),
        vec!["app".to_owned(), "lib".to_owned(), "shared".to_owned()]
    );
}

#[test]
fn remove_usage_prunes_transitive_chain() {
    // root -> b -> c. Removing root's only usage (`b`) must prune both
    // `b` and `c`, since `c` is only reachable through `b`.
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            root_project(Some("me"), "root", &["urn:b"]),
            dep_project("b", "urn:b", &["urn:c"]),
            dep_project("c", "urn:c", &[]),
        ],
    };

    let removed = lock
        .remove_usage(Some("me"), "root", "urn:b")
        .expect("root project should be found");

    assert_eq!(project_names(&lock), vec!["root".to_owned()]);
    let mut removed_names: Vec<_> = removed.iter().map(|p| p.name.clone()).collect();
    removed_names.sort();
    assert_eq!(removed_names, vec!["b".to_owned(), "c".to_owned()]);
}

#[test]
fn remove_usage_keeps_sibling_usage_and_its_own_subtree() {
    // root uses both `b` and `c`, each with their own private dependency.
    // Removing only the `b` usage must prune `b` and its subtree (`onlyb`),
    // while leaving `c` and its subtree (`onlyc`) alone.
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_owned(),
        projects: vec![
            root_project(Some("me"), "root", &["urn:b", "urn:c"]),
            dep_project("b", "urn:b", &["urn:onlyb"]),
            dep_project("onlyb", "urn:onlyb", &[]),
            dep_project("c", "urn:c", &["urn:onlyc"]),
            dep_project("onlyc", "urn:onlyc", &[]),
        ],
    };

    let removed = lock
        .remove_usage(Some("me"), "root", "urn:b")
        .expect("root project should be found");

    let mut removed_names: Vec<_> = removed.iter().map(|p| p.name.clone()).collect();
    removed_names.sort();
    assert_eq!(removed_names, vec!["b".to_owned(), "onlyb".to_owned()]);

    assert_eq!(
        project_names(&lock),
        vec!["c".to_owned(), "onlyc".to_owned(), "root".to_owned()]
    );
    let root = lock.projects.iter().find(|p| p.name == "root").unwrap();
    assert_eq!(
        root.usages.iter().map(Usage::inner).collect::<Vec<_>>(),
        vec!["urn:c"]
    );
}
