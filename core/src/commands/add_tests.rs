// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::{
    add::{AddError, do_add},
    model::{InterchangeProjectInfoRaw, InterchangeProjectUsageRaw},
    project::memory::InMemoryProject,
    utils::format_err,
};
use std::assert_matches;

fn resource(iri: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Resource {
        resource: iri.to_owned(),
        version_constraint: None,
    }
}

fn project() -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "main".to_owned(),
            publisher: Some("publisher".to_owned()),
            description: None,
            version: "1.2.3".to_owned(),
            license: None,
            maintainer: vec![],
            topic: vec![],
            usage: vec![],
            website: None,
        }),
        ..InMemoryProject::default()
    }
}

fn index(publisher: &str, name: &str, constraint: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Index {
        publisher: publisher.to_owned(),
        name: name.to_owned(),
        version_constraint: constraint.to_owned(),
    }
}

fn directory(dir: &str, publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::Directory {
        dir: dir.to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    }
}

fn kpar(kpar_path: &str, publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
    InterchangeProjectUsageRaw::KparPath {
        kpar_path: kpar_path.to_owned(),
        publisher: publisher.to_owned(),
        name: name.to_owned(),
    }
}

fn project_with_usage(usage: InterchangeProjectUsageRaw) -> InMemoryProject {
    let mut project = project();
    project.info.as_mut().unwrap().usage = vec![usage];
    project
}

#[test]
fn add_refuses_a_resource_that_duplicates_a_directory_usage() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    let err = do_add(&mut project, &resource("pkg:sysand/acme-labs/my.project")).unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("already declared as a directory usage"),
        "{message}"
    );
    // The duplicate was not appended.
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_directory_that_duplicates_a_resource_usage() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Resource {
        resource: "pkg:sysand/acme-labs/my.project".to_owned(),
        version_constraint: None,
    });

    let err = do_add(
        &mut project,
        &InterchangeProjectUsageRaw::Directory {
            dir: "../my.project".to_owned(),
            publisher: "acme-labs".to_owned(),
            name: "my.project".to_owned(),
        },
    )
    .unwrap_err();

    let message = format_err(err);
    assert!(
        message.contains("already declared as a resource usage"),
        "{message}"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_kpar_path_that_duplicates_a_directory_usage() {
    // Both are typed, but of different kinds, so neither merge path sees it.
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    let err = do_add(
        &mut project,
        &InterchangeProjectUsageRaw::KparPath {
            kpar_path: "../my.project.kpar".to_owned(),
            publisher: "acme-labs".to_owned(),
            name: "my.project".to_owned(),
        },
    )
    .unwrap_err();

    let message = format_err(err);
    assert!(message.contains("as a KPAR path usage"), "{message}");
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_allows_a_directory_usage_of_a_different_project() {
    let mut project = project_with_usage(InterchangeProjectUsageRaw::Directory {
        dir: "../my.project".to_owned(),
        publisher: "acme-labs".to_owned(),
        name: "my.project".to_owned(),
    });

    do_add(&mut project, &resource("pkg:sysand/acme-labs/other")).unwrap();

    assert_eq!(project.info.unwrap().usage.len(), 2);
}

#[test]
fn add_writes_an_index_usage_as_spelled() {
    let mut project = project();

    assert!(do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap());

    assert_eq!(
        project.info.unwrap().usage,
        [index("Acme Labs", "My Lib", "^1")]
    );
}

/// A different constraint replaces the existing one, as `cargo add` does,
/// so a `*` is never combined with another comparator
#[test]
fn add_replaces_the_constraint_of_a_usage_spelled_the_same() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    assert!(!do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap());
    assert!(do_add(&mut project, &index("Acme Labs", "My Lib", "*")).unwrap());
    assert!(do_add(&mut project, &index("Acme Labs", "My Lib", "^2")).unwrap());

    assert_eq!(
        project.info.unwrap().usage,
        [index("Acme Labs", "My Lib", "^2")]
    );
}

#[test]
fn add_replaces_the_constraint_of_a_resource_usage() {
    let constrained = |vc: &str| InterchangeProjectUsageRaw::Resource {
        resource: "urn:kpar:lib".to_owned(),
        version_constraint: Some(vc.to_owned()),
    };
    let mut project = project_with_usage(constrained("^1"));

    assert!(do_add(&mut project, &constrained("*")).unwrap());
    assert!(do_add(&mut project, &constrained("^2")).unwrap());
    // Without a constraint, the existing one is kept
    assert!(!do_add(&mut project, &resource("urn:kpar:lib")).unwrap());

    assert_eq!(project.info.unwrap().usage, [constrained("^2")]);
}

#[test]
fn add_refuses_an_index_usage_spelled_differently() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    let err = do_add(&mut project, &index("acme labs", "my lib", "^1")).unwrap_err();

    assert_matches!(
        err,
        AddError::TypedUsageSpelledDifferently { kind: "an index", existing, new }
            if existing == "Acme Labs/My Lib" && new == "acme labs/my lib"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_an_index_usage_over_a_legacy_purl() {
    let mut project = project_with_usage(resource("pkg:sysand/acme-labs/my-lib"));

    let err = do_add(&mut project, &index("Acme Labs", "My Lib", "^1")).unwrap_err();

    assert_eq!(
        format_err(err),
        "`pkg:sysand/acme-labs/my-lib` is already declared as a resource usage;\n\
         remove it before adding it as an index usage"
    );
    assert_eq!(project.info.unwrap().usage.len(), 1);
}

#[test]
fn add_refuses_a_legacy_purl_over_an_index_usage() {
    let mut project = project_with_usage(index("Acme Labs", "My Lib", "^1"));

    let err = do_add(&mut project, &resource("pkg:sysand/acme-labs/my-lib")).unwrap_err();

    assert_matches!(
        err,
        AddError::DuplicateIdentifier {
            existing: "an index",
            ..
        }
    );
}

/// Directory and KPAR usages of the same project, spelled differently and
/// from another path, are refused the way index usages are
#[test]
fn add_refuses_a_path_usage_spelled_differently() {
    for (existing, new, kind) in [
        (
            directory("lib", "Acme Labs", "My Lib"),
            directory("other", "acme labs", "my lib"),
            "a directory",
        ),
        (
            kpar("lib.kpar", "Acme Labs", "My Lib"),
            kpar("other.kpar", "acme labs", "my lib"),
            "a KPAR path",
        ),
    ] {
        let mut project = project_with_usage(existing.clone());

        let err = do_add(&mut project, &new).unwrap_err();

        assert_eq!(
            format_err(&err),
            format!(
                "`acme labs/my lib` is already declared as {kind} usage `Acme Labs/My Lib`;\n\
                 a typed usage must spell the publisher and name exactly as the project does"
            )
        );
        assert_eq!(project.info.unwrap().usage, [existing]);
    }
}

/// From the same path, the project was renamed, so the usage takes the new
/// spelling
#[test]
fn add_respells_a_path_usage_from_the_same_path() {
    let mut project = project_with_usage(directory("lib", "Acme Labs", "My Lib"));

    do_add(&mut project, &directory("lib", "acme labs", "my lib")).unwrap();

    assert_eq!(
        project.info.unwrap().usage,
        [directory("lib", "acme labs", "my lib")]
    );
}

mod spell_index_usage {
    use std::{assert_matches, collections::HashMap};

    use crate::{
        add::{IndexSpellingError, spell_index_usage},
        env::memory::MemoryStorageEnvironment,
        model::InterchangeProjectInfoRaw,
        project::{memory::InMemoryProject, utils::Identifier},
    };

    fn installed(publisher: &str, version: &str) -> InMemoryProject {
        InMemoryProject {
            info: Some(InterchangeProjectInfoRaw {
                name: "My Lib".to_owned(),
                publisher: Some(publisher.to_owned()),
                description: None,
                version: version.to_owned(),
                license: None,
                maintainer: vec![],
                topic: vec![],
                usage: vec![],
                website: None,
            }),
            ..InMemoryProject::default()
        }
    }

    fn env(versions: Vec<(&str, InMemoryProject)>) -> MemoryStorageEnvironment<InMemoryProject> {
        MemoryStorageEnvironment {
            projects: HashMap::from([(
                Identifier::from_pub_name("acme-labs", "my-lib").into_string(),
                versions
                    .into_iter()
                    .map(|(version, project)| (version.to_owned(), project))
                    .collect(),
            )]),
        }
    }

    /// The installed spelling, whatever version and constraint
    #[test]
    fn takes_the_installed_spelling() {
        let env = env(vec![("2.0.0", installed("ACME Labs", "2.0.0"))]);
        assert_eq!(
            spell_index_usage(Some(&env), "acme-labs", "my-lib", true).unwrap(),
            ("ACME Labs".to_owned(), "My Lib".to_owned())
        );
        assert_matches!(
            spell_index_usage(Some(&env), "Acme Labs", "My Lib", false),
            Err(IndexSpellingError::Misspelled { spelling, .. }) if spelling == "ACME Labs/My Lib"
        );
    }

    #[test]
    fn fails_without_project_information() {
        let env = env(vec![("1.0.0", InMemoryProject::default())]);
        assert_matches!(
            spell_index_usage(Some(&env), "acme-labs", "my-lib", true),
            Err(IndexSpellingError::MissingInfo { version, .. }) if version == "1.0.0"
        );
    }

    #[test]
    fn fails_when_not_installed() {
        let env = env(vec![]);
        assert_matches!(
            spell_index_usage(Some(&env), "acme-labs", "my-lib", true),
            Err(IndexSpellingError::NotInstalled {
                normalized: true,
                ..
            })
        );
        assert_matches!(
            spell_index_usage::<MemoryStorageEnvironment<InMemoryProject>>(
                None,
                "Acme Labs",
                "My Lib",
                false
            ),
            Err(IndexSpellingError::NotInstalled {
                normalized: false,
                ..
            })
        );
    }
}

mod index_usage_to_add {
    use std::{assert_matches, convert::Infallible};

    use semver::VersionReq;

    use super::index;
    use crate::{
        add::{AddError, IndexUsageToAdd, index_usage_to_add},
        model::InterchangeProjectUsageRaw,
    };

    fn to_add(
        usages: &[InterchangeProjectUsageRaw],
        spelling: (&str, &str),
        constraint: Option<&str>,
    ) -> Result<IndexUsageToAdd, AddError<Infallible>> {
        let (publisher, name) = crate::model::parse_index_usage_spelling(spelling.0, spelling.1)?;
        index_usage_to_add(
            usages,
            publisher,
            name,
            constraint.map(|c| VersionReq::parse(c).unwrap()),
        )
    }

    #[test]
    fn a_project_not_declared_is_new() {
        assert_eq!(
            to_add(&[], ("acme-labs", "my-lib"), None).unwrap(),
            IndexUsageToAdd::New {
                publisher: crate::model::IndexPublisher::parse("acme-labs".to_owned()).unwrap(),
                name: crate::model::IndexName::parse("my-lib".to_owned()).unwrap(),
                version_constraint: None,
                normalized: true,
            }
        );
        assert_matches!(
            to_add(&[], ("Acme Labs", "My Lib"), Some("^1")),
            Ok(IndexUsageToAdd::New {
                normalized: false,
                ..
            })
        );
    }

    #[test]
    fn a_declared_usage_without_a_constraint_is_already_present() {
        let declared = [index("Acme Labs", "My Lib", "^1")];
        for spelling in [("Acme Labs", "My Lib"), ("acme-labs", "my-lib")] {
            assert_eq!(
                to_add(&declared, spelling, None).unwrap(),
                IndexUsageToAdd::AlreadyPresent
            );
        }
    }

    /// With a constraint, the usage is spelled as declared, also when given
    /// normalized
    #[test]
    fn a_declared_usage_with_a_constraint_is_ready_spelled_as_declared() {
        let declared = [index("Acme Labs", "My Lib", "^1")];
        for spelling in [("Acme Labs", "My Lib"), ("acme-labs", "my-lib")] {
            assert_eq!(
                to_add(&declared, spelling, Some("^2")).unwrap(),
                IndexUsageToAdd::Ready(index("Acme Labs", "My Lib", "^2"))
            );
        }
    }

    #[test]
    fn another_spelling_of_a_declared_usage_is_refused() {
        let declared = [index("Acme Labs", "My Lib", "^1")];
        for constraint in [None, Some("^2")] {
            assert_matches!(
                to_add(&declared, ("ACME Labs", "My Lib"), constraint),
                Err(AddError::TypedUsageSpelledDifferently { kind: "an index", existing, new })
                    if existing == "Acme Labs/My Lib" && new == "ACME Labs/My Lib"
            );
        }
    }

    #[test]
    fn a_project_declared_as_another_kind_is_refused() {
        let declared = [InterchangeProjectUsageRaw::Directory {
            dir: "lib".to_owned(),
            publisher: "Acme Labs".to_owned(),
            name: "My Lib".to_owned(),
        }];
        assert_matches!(
            to_add(&declared, ("Acme Labs", "My Lib"), Some("^1")),
            Err(AddError::DuplicateIdentifier {
                existing: "a directory",
                new: "an index",
                ..
            })
        );
    }

    #[test]
    fn a_spelling_no_index_usage_can_have_is_refused() {
        assert_matches!(
            to_add(&[], ("A", "My Lib"), Some("^1")),
            Err(AddError::Validation(_))
        );
    }
}
