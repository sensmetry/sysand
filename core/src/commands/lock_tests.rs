// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::collections::HashMap;

use indexmap::IndexMap;

use crate::{
    commands::lock::{LockError, do_lock_extend, do_lock_projects},
    context::ProjectContext,
    lock::{Lock, Project, Source},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::memory::InMemoryProject,
    resolve::null::NullResolver,
    solve::pubgrub::SolveOptions,
};

#[test]
fn lock_export_conflict() {
    let exports = vec!["sym1".into(), "sym2".into(), "sym3".into()];

    let lock = Lock {
        lock_version: String::new(),
        projects: vec![
            Project {
                name: "test1".into(),
                publisher: None,
                version: String::new(),
                exports: exports.clone(),
                identifiers: vec!["test1".into()],
                sources: vec![],
                usages: vec![],
            },
            Project {
                name: "test2".into(),
                publisher: None,
                version: String::new(),
                exports,
                identifiers: vec!["test2".into()],
                sources: vec![],
                usages: vec![],
            },
        ],
    };
    let res = do_lock_extend(
        lock,
        [],
        NullResolver {},
        SolveOptions::default(),
        &HashMap::new(),
        &ProjectContext::default(),
    );

    assert_matches!(res, Err(LockError::NameCollision(_)));
}

#[test]
fn lock_preserves_project_publisher() {
    let mut project = InMemoryProject::from_info_meta(
        InterchangeProjectInfoRaw {
            name: "published_project".into(),
            publisher: Some("Acme Labs".into()),
            version: "1.2.3".into(),
            description: None,
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        },
        InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: "2026-01-01T00:00:00Z".into(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        },
    );
    project.nominal_sources = vec![Source::Editable {
        editable: ".".into(),
    }];

    let lock = do_lock_projects(
        [(None, &project)],
        NullResolver {},
        SolveOptions::default(),
        &HashMap::new(),
        &ProjectContext::default(),
    )
    .unwrap()
    .lock;

    assert_eq!(lock.projects[0].publisher.as_deref(), Some("Acme Labs"));
}

/// The root wants `library >=0.11.0, <0.12.0`, a dependent it also uses
/// pins `library "0.10.1"` (caret), and both versions exist: the failure
/// must name the dependent as the party that excludes the wanted version.
#[test]
fn lock_reports_which_dependent_pins_a_conflicting_version() {
    use crate::{
        model::InterchangeProjectUsageRaw,
        project::utils::Identifier,
        resolve::memory::{AcceptAll, MemoryResolver},
        solve::pubgrub::SolveConflict,
    };

    const LIBRARY: &str = "pkg:sysand/mock/library";
    const DEPENDENT: &str = "pkg:sysand/mock/dependent";

    fn remote(name: &str, version: &str, usage: Vec<(&str, &str)>) -> InMemoryProject {
        InMemoryProject::from_info_meta(
            InterchangeProjectInfoRaw {
                name: name.into(),
                publisher: None,
                version: version.into(),
                description: None,
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: usage
                    .into_iter()
                    .map(
                        |(resource, constraint)| InterchangeProjectUsageRaw::Resource {
                            resource: resource.into(),
                            version_constraint: Some(constraint.into()),
                        },
                    )
                    .collect(),
            },
            InterchangeProjectMetadataRaw {
                index: IndexMap::default(),
                created: "2026-01-01T00:00:00Z".into(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: None,
            },
        )
    }

    let mut root = remote(
        "migrating",
        "1.0.0",
        vec![
            (DEPENDENT, ">=1.0.0, <2.0.0"),
            (LIBRARY, ">=0.11.0, <0.12.0"),
        ],
    );
    root.nominal_sources = vec![Source::Editable {
        editable: ".".into(),
    }];
    let resolver = MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::from([
            (
                Identifier::from_iri_unchecked_str(DEPENDENT),
                vec![remote("dependent", "1.0.0", vec![(LIBRARY, "0.10.1")])],
            ),
            (
                Identifier::from_iri_unchecked_str(LIBRARY),
                vec![
                    remote("library", "0.10.3", vec![]),
                    remote("library", "0.11.0", vec![]),
                ],
            ),
        ]),
    };

    let err = do_lock_projects(
        [(None, &root)],
        resolver,
        SolveOptions::default(),
        &HashMap::new(),
        &ProjectContext::default(),
    )
    .unwrap_err();

    let crate::commands::lock::LockProjectError::LockError(LockError::Solver(err)) = err else {
        panic!("expected a solver failure, got {err}");
    };
    let conflicts = err.conflicts();
    assert!(
        conflicts.contains(&SolveConflict::Constraint {
            iri: LIBRARY.to_owned(),
            constraint: "^0.10.1".to_owned(),
            required_by: Some(DEPENDENT.to_owned()),
        }),
        "the dependent's pin must be named: {conflicts:?}"
    );
    assert!(
        conflicts.contains(&SolveConflict::Constraint {
            iri: LIBRARY.to_owned(),
            constraint: ">=0.11.0, <0.12.0".to_owned(),
            required_by: None,
        }),
        "the root's own constraint must be named: {conflicts:?}"
    );
}

/// An index usage must spell the publisher and name of the project it
/// resolves to exactly as that project does
mod index_usage_spelling {
    use super::*;
    use crate::{
        commands::lock::{DeclaredBy, IndexUsageMismatchError, LockProjectError},
        model::{IndexUsage, InterchangeProjectUsageRaw},
        project::utils::Identifier,
        resolve::memory::{AcceptAll, MemoryResolver},
    };

    fn project(
        publisher: Option<&str>,
        name: &str,
        usage: Vec<InterchangeProjectUsageRaw>,
    ) -> InMemoryProject {
        let mut project = InMemoryProject::from_info_meta(
            InterchangeProjectInfoRaw {
                name: name.into(),
                publisher: publisher.map(Into::into),
                version: "1.0.0".into(),
                description: None,
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage,
            },
            InterchangeProjectMetadataRaw {
                index: IndexMap::default(),
                created: "2026-01-01T00:00:00Z".into(),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: None,
            },
        );
        project.nominal_sources = vec![Source::Editable {
            editable: ".".into(),
        }];
        project
    }

    fn index(publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher: publisher.into(),
            name: name.into(),
            version_constraint: "^1".into(),
        })
    }

    /// Lock `root` against `projects`, keyed by their normalized identifiers
    fn lock(
        root: &InMemoryProject,
        projects: Vec<(&str, &str, InMemoryProject)>,
    ) -> Result<Lock, Box<IndexUsageMismatchError>> {
        let resolver = MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: projects
                .into_iter()
                .map(|(publisher, name, project)| {
                    (Identifier::from_pub_name(publisher, name), vec![project])
                })
                .collect(),
        };
        match do_lock_projects(
            [(None, root)],
            resolver,
            SolveOptions::default(),
            &HashMap::new(),
            &ProjectContext::default(),
        ) {
            Ok(outcome) => Ok(outcome.lock),
            Err(LockProjectError::LockError(LockError::IndexUsageMismatch(e))) => Err(e),
            Err(e) => panic!("{e}"),
        }
    }

    fn root(usage: InterchangeProjectUsageRaw) -> InMemoryProject {
        project(Some("Me"), "app", vec![usage])
    }

    #[test]
    fn exact_spelling_locks() {
        let lib = project(Some("Acme Labs"), "My Lib", vec![]);
        let lock = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap();
        assert_eq!(lock.projects.len(), 2);
    }

    #[test]
    fn publisher_mismatch() {
        let lib = project(Some("acme-labs"), "My Lib", vec![]);
        let err = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert_eq!(err.publisher.as_deref(), Some("acme-labs"));
        assert_eq!(err.declared_by, DeclaredBy::Input("`app` 1.0.0".to_owned()));
        assert_eq!(
            err.to_string(),
            "index usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 \
             of a project that declares itself `acme-labs/My Lib`;\n\
             spell the usage as the project does"
        );
    }

    #[test]
    fn name_mismatch_in_case_only() {
        let lib = project(Some("Acme Labs"), "my lib", vec![]);
        let err = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert_eq!(err.name, "my lib");
    }

    #[test]
    fn target_without_publisher() {
        let lib = project(None, "My Lib", vec![]);
        let err = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("declares itself `<none>/My Lib`"),
            "{err}"
        );
    }

    #[test]
    fn mismatch_in_a_dependency() {
        let mid = project(Some("Acme"), "Mid", vec![index("acme", "lib")]);
        let lib = project(Some("Acme"), "Lib", vec![]);
        let err = lock(
            &root(index("Acme", "Mid")),
            vec![("acme", "mid", mid), ("acme", "lib", lib)],
        )
        .unwrap_err();
        assert_eq!(
            err.declared_by,
            DeclaredBy::Dependency("`Mid` 1.0.0 (`pkg:sysand/acme/mid`)".to_owned())
        );
        assert!(
            err.to_string()
                .ends_with("it has to be fixed by that dependency's publisher"),
            "{err}"
        );
    }
}
