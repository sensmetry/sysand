// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{fmt::Display, num::NonZeroU64, slice, str::FromStr};

use toml_edit::DocumentMut;
use typed_path::Utf8UnixPathBuf;

use crate::lock::{
    CURRENT_LOCK_VERSION, LOCKFILE_PREFIX, Lock, Project, Source, Usage, ValidationError,
    VersionError, check_lock_version,
};

#[test]
fn check_current_lock_version() {
    let version = CURRENT_LOCK_VERSION.to_string();
    let document =
        DocumentMut::from_str(format!(r#"lock_version = "{}""#, version).as_str()).unwrap();
    check_lock_version(&document).unwrap();
}

#[test]
fn check_unsupported_lock_version() {
    let version = "X";
    let document =
        DocumentMut::from_str(format!(r#"lock_version = "{}""#, version).as_str()).unwrap();
    let Err(err) = check_lock_version(&document) else {
        panic!()
    };
    let VersionError::Unsupported(ref s) = err else {
        panic!()
    };
    assert_eq!(s, version);
    assert_eq!(
        err.to_string(),
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
    let crate::lock::ParseError::Version(VersionError::Unsupported(ref s)) = err else {
        panic!("expected unsupported version error, got {err:?}")
    };
    assert_eq!(s, "0.3");
    assert_eq!(
        err.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects,
    };
    let expected = format!(
        "{}lock_version = \"{}\"\n{}",
        LOCKFILE_PREFIX, CURRENT_LOCK_VERSION, toml
    );
    assert_eq!(lock.to_string(), expected.to_string());
}

#[test]
fn minimal_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "a".to_owned(),
            publisher: None,
            version: "0.0.1".to_string(),
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
                name: "One".to_string(),
                publisher: None,
                version: "0.0.1".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
            Project {
                name: "Two".to_string(),
                publisher: None,
                version: "0.0.2".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
            Project {
                name: "Three".to_string(),
                publisher: None,
                version: "0.0.3".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
            },
        ],
        r#"
[[project]]
name = "One"
version = "0.0.1"

[[project]]
name = "Two"
version = "0.0.2"

[[project]]
name = "Three"
version = "0.0.3"
"#,
    );
}

#[test]
fn one_export_to_toml() {
    to_toml_matches_expected(
        vec![Project {
            name: "One Package".to_string(),
            publisher: None,
            version: "0.1.1".to_string(),
            exports: vec!["PackageName".to_string()],
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
            name: "Three Packages".to_string(),
            publisher: None,
            version: "0.1.3".to_string(),
            exports: vec![
                "Package1".to_string(),
                "Package2".to_string(),
                "Package3".to_string(),
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
            name: "One IRI".to_string(),
            publisher: None,
            version: "0.2.1".to_string(),
            exports: vec![],
            identifiers: vec!["urn:kpar:example".to_string()],
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
            name: "Three IRI:s".to_string(),
            publisher: None,
            version: "0.2.3".to_string(),
            exports: vec![],
            identifiers: vec![
                "urn:kpar:example".to_string(),
                "ftp://www.example.com".to_string(),
                "http://www.example.com".to_string(),
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
            name: "One source".to_string(),
            publisher: None,
            version: "0.4.1".to_string(),
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
            name: "Seven sources".to_string(),
            publisher: None,
            version: "0.4.7".to_string(),
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
                        .to_string(),
                },
                Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("example/path"),
                    checksum: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                },
                Source::RemoteKpar {
                    remote_kpar: "www.example.com/remote.kpar".to_string(),
                    kpar_size: NonZeroU64::new(64).unwrap(),
                    kpar_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                },
                Source::IndexKpar {
                    index_kpar: "www.example.com/index.kpar".to_string(),
                    kpar_size: NonZeroU64::new(128).unwrap(),
                    kpar_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                },
                Source::RemoteSrc {
                    remote_src: "www.example.com/remote".to_string(),
                    checksum: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                },
                Source::RemoteGit {
                    remote_git: "github.com/example/remote.git".to_string(),
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
            name: "One usage".to_string(),
            publisher: None,
            version: "0.5.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![Usage {
                resource: "urn:kpar:usage".to_string(),
            }],
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
            name: "Three usages".to_string(),
            publisher: None,
            version: "0.5.3".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![
                Usage {
                    resource: "urn:kpar:first".to_string(),
                },
                Usage {
                    resource: "urn:kpar:second".to_string(),
                },
                Usage {
                    resource: "urn:kpar:third".to_string(),
                },
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
    let expected = format!(
        "{}lock_version = \"{}\"\n{}",
        LOCKFILE_PREFIX, CURRENT_LOCK_VERSION, toml
    );
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
        version: version.as_ref().to_string(),
        exports: exports.iter().map(|s| String::from(*s)).collect(),
        identifiers: identifiers.iter().map(|s| String::from(*s)).collect(),
        usages: usages.to_vec(),
        sources: vec![],
    }
}

#[test]
fn validate_empty() {
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_minimal() {
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![make_project("a", None, "0.0.1", &[], &[], &[])],
    }
    .validate()
    .unwrap();
}

#[test]
fn validate_single_usage() {
    let iri = "urn:kpar:test";
    Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[
                    Usage {
                        resource: iri1.to_string(),
                    },
                    Usage {
                        resource: iri2.to_string(),
                    },
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage {
                    resource: iri1.to_string(),
                }],
            ),
            make_project(
                "b",
                None,
                "0.0.1",
                &[],
                &[iri1],
                &[Usage {
                    resource: iri2.to_string(),
                }],
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
    let ValidationError::UnsupportedVersion(ref s) = err else {
        panic!()
    };
    assert_eq!(s, version);
    assert_eq!(
        err.to_string(),
        "lockfile version `X` is not supported; regenerate it with a lock operation"
    );
}

#[test]
fn validate_single_name_collision() {
    let name = "PackageName";
    let iri = "urn:kpar:test";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[name],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                "a",
                None,
                "0.0.1",
                &[name1, name2, name3],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
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
    let usage_in = Usage {
        resource: "urn:kpar:test".to_string(),
    };
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
    assert_eq!(usage, usage_in.resource);
    assert_eq!(name, "a");
}

#[test]
fn validate_checksum() {
    let invalid_checksum = "dA8747a6f27A32f10Ba393113bCE29fX88181037a71f093f90e0ad5829D2b780";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![Project {
            name: "a".into(),
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![Source::LocalSrc {
                src_path: Utf8UnixPathBuf::from("../path/to/the/project"),
                checksum: invalid_checksum.to_owned(),
            }],
        }],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::InvalidProjectDigestFormat { digest, name } = err else {
        panic!()
    };
    assert_eq!(digest, invalid_checksum);
    assert_eq!(name, "a");
}

#[test]
fn validate_kpar_digest_rejects_uppercase() {
    let invalid_digest = "dA8747a6f27A32f10Ba393113bCe29f788181037a71f093f90e0ad5829d2b780";
    let err = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![Project {
            name: "Indexed".to_string(),
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec!["urn:kpar:indexed".to_string()],
            usages: vec![],
            sources: vec![Source::IndexKpar {
                index_kpar: "https://example.com/indexed.kpar".to_string(),
                kpar_size: std::num::NonZeroU64::new(123).unwrap(),
                kpar_digest: invalid_digest.to_string(),
            }],
        }],
    }
    .validate()
    .unwrap_err();
    let ValidationError::InvalidKparDigestFormat {
        digest,
        project_with_name,
    } = err
    else {
        panic!()
    };
    assert_eq!(digest, invalid_digest);
    assert_eq!(project_with_name, "urn:kpar:indexed");
}

#[test]
fn sort_empty() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

#[test]
fn sort_identifiers() {
    let project1 = make_project("a", None, "0.0.1", &[], &["urn:kpar:b", "urn:kpar:a"], &[]);
    let project2 = make_project("a", None, "0.0.1", &[], &["urn:kpar:a", "urn:kpar:b"], &[]);
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

#[test]
fn sort_sources() {
    let usage1 = Usage {
        resource: "urn:kpar:a".to_string(),
    };
    let usage2 = Usage {
        resource: "urn:kpar:b".to_string(),
    };
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![project1],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project2]);
}

#[test]
fn sort_sources_with_constraints() {
    let usage1 = Usage {
        resource: "urn:kpar:a".to_string(),
    };
    let usage2 = Usage {
        resource: "urn:kpar:a".to_string(),
    };
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
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
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![project2.clone(), project1.clone()],
    };
    lock.sort();
    let Lock { projects, .. } = lock;
    assert_eq!(projects, vec![project1, project2]);
}

#[test]
fn canonicalize_checksums() {
    let mut lock = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![Project {
            name: "a".into(),
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![Source::LocalSrc {
                src_path: Utf8UnixPathBuf::from("../path/to/the/project"),
                checksum: "dA8747a6f27A32f10Ba393113bCE29f788181037a71f093f90e0ad5829D2b780"
                    .to_owned(),
            }],
        }],
    };
    lock.canonicalize_checksums();
    let Lock { projects, .. } = lock;
    let [project] = projects.as_slice() else {
        panic!()
    };
    let Source::LocalSrc {
        src_path: _,
        checksum,
    } = &project.sources[0]
    else {
        panic!()
    };
    assert_eq!(
        checksum,
        "da8747a6f27a32f10ba393113bce29f788181037a71f093f90e0ad5829d2b780"
    );
}
