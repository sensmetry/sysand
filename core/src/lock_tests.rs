// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{convert::Infallible, fmt::Display, slice, str::FromStr};

use toml_edit::DocumentMut;
use typed_path::Utf8UnixPathBuf;

use crate::lock::{
    CURRENT_LOCK_VERSION, LOCKFILE_PREFIX, Lock, Project, Source, Usage, ValidationError,
    VersionError, check_lock_version, project_with,
};

const CHECKSUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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
    let VersionError::Unsupported(s) = err else {
        panic!()
    };
    assert_eq!(s, version);
}

#[test]
fn check_missing_lock_version() {
    let document = DocumentMut::from_str("").unwrap();
    let Err(err) = check_lock_version(&document) else {
        panic!()
    };
    let VersionError::Missing = err else { panic!() };
}

fn test_to_toml<D: Display>(projects: Vec<Project>, toml: D) {
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
    test_to_toml(
        vec![Project {
            name: None,
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
version = "0.0.1"
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn many_projects_to_toml() {
    test_to_toml(
        vec![
            Project {
                name: Some("One".to_string()),
                publisher: None,
                version: "0.0.1".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
                checksum: CHECKSUM.to_string(),
            },
            Project {
                name: Some("Two".to_string()),
                publisher: None,
                version: "0.0.2".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
                checksum: CHECKSUM.to_string(),
            },
            Project {
                name: Some("Three".to_string()),
                publisher: None,
                version: "0.0.3".to_string(),
                exports: vec![],
                identifiers: vec![],
                usages: vec![],
                sources: vec![],
                checksum: CHECKSUM.to_string(),
            },
        ],
        format!(
            r#"
[[project]]
name = "One"
version = "0.0.1"
checksum = "{CHECKSUM}"

[[project]]
name = "Two"
version = "0.0.2"
checksum = "{CHECKSUM}"

[[project]]
name = "Three"
version = "0.0.3"
checksum = "{CHECKSUM}"
"#,
        ),
    );
}

#[test]
fn one_export_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("One Package".to_string()),
            publisher: None,
            version: "0.1.1".to_string(),
            exports: vec!["PackageName".to_string()],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "One Package"
version = "0.1.1"
exports = [
    "PackageName",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn many_exports_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("Three Packages".to_string()),
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
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "Three Packages"
version = "0.1.3"
exports = [
    "Package1",
    "Package2",
    "Package3",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn one_iri_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("One IRI".to_string()),
            publisher: None,
            version: "0.2.1".to_string(),
            exports: vec![],
            identifiers: vec!["urn:kpar:example".to_string()],
            usages: vec![],
            sources: vec![],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "One IRI"
version = "0.2.1"
identifiers = [
    "urn:kpar:example",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn many_identifiers_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("Three IRI:s".to_string()),
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
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "Three IRI:s"
version = "0.2.3"
identifiers = [
    "urn:kpar:example",
    "ftp://www.example.com",
    "http://www.example.com",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn one_source_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("One source".to_string()),
            publisher: None,
            version: "0.4.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![Source::Editable {
                editable: Utf8UnixPathBuf::from("."),
            }],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "One source"
version = "0.4.1"
sources = [
    {{ editable = "." }},
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn many_sources_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("Seven sources".to_string()),
            publisher: None,
            version: "0.4.7".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![
                Source::LocalKpar {
                    kpar_path: Utf8UnixPathBuf::from("example.kpar"),
                },
                Source::LocalSrc {
                    src_path: Utf8UnixPathBuf::from("example/path"),
                },
                Source::Registry {
                    registry: "www.example.com".to_string(),
                },
                Source::RemoteKpar {
                    remote_kpar: "www.example.com/remote.kpar".to_string(),
                    remote_kpar_size: Some(64),
                },
                Source::RemoteSrc {
                    remote_src: "www.example.com/remote".to_string(),
                },
                Source::RemoteGit {
                    remote_git: "github.com/example/remote.git".to_string(),
                },
                Source::RemoteApi {
                    remote_api: "www.example.com/api".to_string(),
                },
            ],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "Seven sources"
version = "0.4.7"
sources = [
    {{ kpar_path = "example.kpar" }},
    {{ src_path = "example/path" }},
    {{ registry = "www.example.com" }},
    {{ remote_kpar = "www.example.com/remote.kpar", remote_kpar_size = 64 }},
    {{ remote_src = "www.example.com/remote" }},
    {{ remote_git = "github.com/example/remote.git" }},
    {{ remote_api = "www.example.com/api" }},
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn one_usage_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("One usage".to_string()),
            publisher: None,
            version: "0.5.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![Usage {
                resource: "urn:kpar:usage".to_string(),
            }],
            sources: vec![],
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "One usage"
version = "0.5.1"
usages = [
    "urn:kpar:usage",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

#[test]
fn many_usage_to_toml() {
    test_to_toml(
        vec![Project {
            name: Some("Three usages".to_string()),
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
            checksum: CHECKSUM.to_string(),
        }],
        format!(
            r#"
[[project]]
name = "Three usages"
version = "0.5.3"
usages = [
    "urn:kpar:first",
    "urn:kpar:second",
    "urn:kpar:third",
]
checksum = "{CHECKSUM}"
"#
        ),
    );
}

fn test_roundtrip<D: Display>(toml: D) {
    let expected = format!(
        "{}lock_version = \"{}\"\n{}",
        LOCKFILE_PREFIX, CURRENT_LOCK_VERSION, toml
    );
    let lockfile: Lock = toml::from_str(&expected).unwrap();
    assert_eq!(lockfile.to_string(), expected);
}

#[test]
fn simple_roundtrip() {
    test_roundtrip(format!(
        r#"
[[project]]
name = "Simple"
version = "0.0.1"
checksum = "{CHECKSUM}"
"#
    ));
}

#[test]
fn complex_roundtrip() {
    test_roundtrip(format!(
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
checksum = "{CHECKSUM}"

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
checksum = "{CHECKSUM}"

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
checksum = "{CHECKSUM}"
"#
    ));
}

fn make_project<S: AsRef<str>>(
    name: Option<String>,
    publisher: Option<String>,
    version: S,
    exports: &[&'static str],
    identifiers: &[&'static str],
    usages: &[Usage],
) -> Project {
    Project {
        name,
        publisher,
        version: version.as_ref().to_string(),
        exports: exports.iter().map(|s| String::from(*s)).collect(),
        identifiers: identifiers.iter().map(|s| String::from(*s)).collect(),
        usages: usages.to_vec(),
        sources: vec![],
        checksum: CHECKSUM.to_string(),
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
        projects: vec![make_project(None, None, "0.0.1", &[], &[], &[])],
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
                None,
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
            ),
            make_project(None, None, "0.0.1", &[], &[iri], &[]),
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
                None,
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
            make_project(None, None, "0.0.1", &[], &[iri1], &[]),
            make_project(None, None, "0.0.1", &[], &[iri2], &[]),
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
                None,
                None,
                "0.0.1",
                &[],
                &[],
                &[Usage {
                    resource: iri1.to_string(),
                }],
            ),
            make_project(
                None,
                None,
                "0.0.1",
                &[],
                &[iri1],
                &[Usage {
                    resource: iri2.to_string(),
                }],
            ),
            make_project(None, None, "0.0.1", &[], &[iri2], &[]),
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
    let ValidationError::UnsupportedVersion(s) = err else {
        panic!()
    };
    assert_eq!(s, version);
}

#[test]
fn validate_single_name_collision() {
    let name = "PackageName";
    let iri = "urn:kpar:test";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![
            make_project(
                None,
                None,
                "0.0.1",
                &[name],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
            ),
            make_project(None, None, "0.0.1", &[name], &[iri], &[]),
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
                None,
                None,
                "0.0.1",
                &[name1, name2, name3],
                &[],
                &[Usage {
                    resource: iri.to_string(),
                }],
            ),
            make_project(None, None, "0.0.1", &[name2, name3, name4], &[iri], &[]),
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
            None,
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
    let ValidationError::UnsatisfiedUsage {
        usage,
        project_with_name,
    } = err
    else {
        panic!()
    };
    assert_eq!(usage, usage_in.resource);
    assert_eq!(project_with_name, project_with::<String>(None));
}

#[test]
fn validate_checksum() {
    let invalid_checksum = "dA8747a6f27A32f10Ba393113bCE29fX88181037a71f093f90e0ad5829D2b780";
    let Err(err) = Lock {
        lock_version: CURRENT_LOCK_VERSION.to_string(),
        projects: vec![Project {
            name: None,
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
            checksum: invalid_checksum.to_owned(),
        }],
    }
    .validate() else {
        panic!()
    };
    let ValidationError::InvalidChecksumFormat {
        checksum,
        project_with_name,
    } = err
    else {
        panic!()
    };
    assert_eq!(checksum, invalid_checksum);
    assert_eq!(project_with_name, project_with::<Infallible>(None));
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
    let project = make_project(None, None, "0.0.1", &[], &[], &[]);
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
    let project1 = make_project(None, None, "0.0.1", &["B", "A"], &[], &[]);
    let project2 = make_project(None, None, "0.0.1", &["A", "B"], &[], &[]);
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
    let project1 = make_project(None, None, "0.0.1", &[], &["urn:kpar:b", "urn:kpar:a"], &[]);
    let project2 = make_project(None, None, "0.0.1", &[], &["urn:kpar:a", "urn:kpar:b"], &[]);
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
        None,
        None,
        "0.0.1",
        &[],
        &[],
        &[usage2.clone(), usage1.clone()],
    );
    let project2 = make_project(None, None, "0.0.1", &[], &[], &[usage1, usage2]);
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
        None,
        None,
        "0.0.1",
        &[],
        &[],
        &[usage2.clone(), usage1.clone()],
    );
    let project2 = make_project(None, None, "0.0.1", &[], &[], &[usage1, usage2]);
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
    let project1 = make_project(
        Some("A".to_string()),
        None,
        "0.0.2",
        &["B"],
        &["urn:kpar:b"],
        &[],
    );
    let project2 = make_project(
        Some("B".to_string()),
        None,
        "0.0.1",
        &["A"],
        &["urn:kpar:a"],
        &[],
    );
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
    let project1 = make_project(
        Some("A".to_string()),
        None,
        "0.0.2",
        &["A"],
        &["urn:kpar:b"],
        &[],
    );
    let project2 = make_project(
        Some("A".to_string()),
        None,
        "0.0.1",
        &["B"],
        &["urn:kpar:a"],
        &[],
    );
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
    let project1 = make_project(
        Some("A".to_string()),
        None,
        "0.0.2",
        &["A"],
        &["urn:kpar:a"],
        &[],
    );
    let project2 = make_project(
        Some("A".to_string()),
        None,
        "0.0.1",
        &["A"],
        &["urn:kpar:b"],
        &[],
    );
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
    let project1 = make_project(
        Some("A".to_string()),
        None,
        "0.0.1",
        &["A"],
        &["urn:kpar:a"],
        &[],
    );
    let project2 = make_project(
        Some("A".to_string()),
        None,
        "0.0.2",
        &["A"],
        &["urn:kpar:a"],
        &[],
    );
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
            name: None,
            publisher: None,
            version: "0.0.1".to_string(),
            exports: vec![],
            identifiers: vec![],
            usages: vec![],
            sources: vec![],
            checksum: "dA8747a6f27A32f10Ba393113bCE29f788181037a71f093f90e0ad5829D2b780"
                .to_string(),
        }],
    };
    lock.canonicalize_checksums();
    let Lock { projects, .. } = lock;
    let [project] = projects.as_slice() else {
        panic!()
    };
    assert_eq!(
        project.checksum,
        "da8747a6f27a32f10ba393113bce29f788181037a71f093f90e0ad5829d2b780"
    );
}
