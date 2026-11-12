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
};

#[cfg(feature = "filesystem")]
use std::path::Path;

#[cfg(feature = "filesystem")]
use camino::Utf8Path;
#[cfg(feature = "filesystem")]
use url::Url;

#[cfg(feature = "filesystem")]
use crate::commands::lock::LockProjectError;
#[cfg(feature = "filesystem")]
use crate::model::InterchangeProjectUsage;
#[cfg(feature = "filesystem")]
use crate::resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead};

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
                metamodel: None,
                exports: exports.clone(),
                identifiers: vec!["test1".into()],
                sources: vec![],
                usages: vec![],
            },
            Project {
                name: "test2".into(),
                publisher: None,
                version: String::new(),
                metamodel: None,
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
            index: Some(IndexMap::default()),
            root: None,
            exports: None,
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
        &HashMap::new(),
        &ProjectContext::default(),
    )
    .unwrap();

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
                index: Some(IndexMap::default()),
                root: None,
                exports: None,
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

/// A typed usage must spell the publisher and name of the project it
/// resolves to exactly as that project does
mod typed_usage_spelling {
    use super::*;
    use crate::{
        commands::lock::{DeclaredBy, LockProjectError, TypedUsageMismatchError},
        model::InterchangeProjectUsageRaw,
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
                index: Some(IndexMap::default()),
                root: None,
                exports: None,
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
        InterchangeProjectUsageRaw::Index {
            publisher: publisher.into(),
            name: name.into(),
            version_constraint: "^1".into(),
        }
    }

    fn directory(publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::Directory {
            dir: "lib".into(),
            publisher: publisher.into(),
            name: name.into(),
        }
    }

    fn kpar(publisher: &str, name: &str) -> InterchangeProjectUsageRaw {
        InterchangeProjectUsageRaw::KparPath {
            kpar_path: "lib.kpar".into(),
            publisher: publisher.into(),
            name: name.into(),
        }
    }

    /// Lock `root` against `projects`, keyed by their normalized identifiers
    fn lock(
        root: &InMemoryProject,
        projects: Vec<(&str, &str, InMemoryProject)>,
    ) -> Result<Lock, Box<TypedUsageMismatchError>> {
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
            &HashMap::new(),
            &ProjectContext::default(),
        ) {
            Ok(lock) => Ok(lock),
            Err(LockProjectError::LockError(LockError::TypedUsageMismatch(e))) => Err(e),
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
            "an index usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 \
             of `acme-labs/My Lib`, but is rejected because its spelling does not match the \
             project's;\nspell the usage exactly as `acme-labs/My Lib`"
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
            &root(kpar("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "a KPAR path usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 of \
             `My Lib`, which declares no publisher, so no typed usage can name it;\n\
             use a resource usage of it instead (e.g. `sysand add --iri <IRI>`)"
        );
    }

    /// An index entry that declares another identity than the one it is
    /// served under is the index's error, not a spelling to adopt
    #[test]
    fn index_entry_declaring_another_project() {
        let lib = project(Some("Globex"), "My Lib", vec![]);
        let err = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert!(err.invalid_index_entry);
        assert_eq!(
            err.to_string(),
            "an index usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 of \
             `Globex/My Lib`, which is not the project the usage names;\n\
             the index entry for `Acme Labs/My Lib` is invalid and has to be fixed by the \
             index's maintainers"
        );
    }

    #[test]
    fn index_entry_without_publisher() {
        let lib = project(None, "My Lib", vec![]);
        let err = lock(
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert!(err.invalid_index_entry);
        assert!(
            err.to_string()
                .contains("of `<none>/My Lib`, which is not the project"),
            "{err}"
        );
    }

    /// A path usage's target declaring another identity is still a
    /// spelling to adopt
    #[test]
    fn path_target_declaring_another_project() {
        let lib = project(Some("Globex"), "My Lib", vec![]);
        let err = lock(
            &root(kpar("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert!(!err.invalid_index_entry);
        assert!(
            err.to_string()
                .ends_with("spell the usage exactly as `Globex/My Lib`"),
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

    /// Directory and KPAR usages are checked the same way as index usages
    #[test]
    fn path_usages_are_checked() {
        for (usage, kind) in [
            (directory("Acme Labs", "My Lib"), "a directory"),
            (kpar("Acme Labs", "My Lib"), "a KPAR path"),
        ] {
            let lib = project(Some("Acme Labs"), "My Lib", vec![]);
            lock(&root(usage.clone()), vec![("acme labs", "my lib", lib)]).unwrap();

            let lib = project(Some("acme-labs"), "My Lib", vec![]);
            let err = lock(&root(usage), vec![("acme labs", "my lib", lib)]).unwrap_err();
            assert_eq!(err.kind, kind);
            assert!(
                err.to_string().starts_with(&format!(
                    "{kind} usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 \
                     of `acme-labs/My Lib`"
                )),
                "{err}"
            );
        }
    }

    #[test]
    fn path_usage_mismatch_in_a_dependency() {
        let mid = project(Some("Acme"), "Mid", vec![directory("Acme", "Lib")]);
        let lib = project(Some("Acme"), "lib", vec![]);
        let err = lock(
            &root(index("Acme", "Mid")),
            vec![("acme", "mid", mid), ("acme", "lib", lib)],
        )
        .unwrap_err();
        assert_eq!(err.kind, "a directory");
        assert_eq!(
            err.declared_by,
            DeclaredBy::Dependency("`Mid` 1.0.0 (`pkg:sysand/acme/mid`)".to_owned())
        );
    }
}

/// A resolver that records every usage it is asked to resolve, and
/// resolves nothing: the solve fails after the first usage, which is
/// enough to see what was asked.
#[cfg(feature = "filesystem")]
#[derive(Debug, Default)]
struct RecordingResolver {
    seen: std::rc::Rc<std::cell::RefCell<Vec<InterchangeProjectUsage>>>,
}

#[cfg(feature = "filesystem")]
impl ResolveRead for RecordingResolver {
    type Error = !;

    type ProjectStorage = crate::project::null::NullProject;

    type ResolvedStorages = std::iter::Empty<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.seen.borrow_mut().push(resolve.usage().clone());
        Ok(ResolutionOutcome::NotFound {
            reason: "recording resolver".to_owned(),
        })
    }
}

/// A `model.toml` `kpar-url` usage reaches the resolver as a typed KPAR
/// URL usage, carrying the publisher and name it declares.
#[cfg(feature = "filesystem")]
#[test]
fn a_toml_kpar_url_usage_is_requested_as_a_typed_kpar_url_usage() {
    use crate::project::local_src::LocalSrcProject;

    let toml = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        toml.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"app\"\n\
         version = \"1.0.0\"\n\
         sources = []\n\
         metamodel = \"https://www.omg.org/spec/SysML/20250201\"\n\n\
         [[inline-usages]]\n\
         publisher = \"acme\"\n\
         name = \"lib\"\n\
         kpar-url = \"https://example.com/lib.kpar\"\n",
    )
    .expect("write model.toml");
    let project = crate::project::editable::EditableProject::new(
        typed_path::Utf8UnixPathBuf::from("."),
        LocalSrcProject::new_access(toml.path(), None),
    );
    let resolver = RecordingResolver::default();
    let seen = resolver.seen.clone();
    do_lock_projects(
        [(None, &project)],
        resolver,
        &HashMap::new(),
        &ProjectContext::default(),
    )
    .expect_err("the resolver finds nothing");
    let seen = seen.borrow();
    let [
        InterchangeProjectUsage::RemoteKpar {
            kpar_url,
            publisher: Some(publisher),
            name,
        },
    ] = seen.as_slice()
    else {
        panic!("expected one typed remote KPAR usage, got {seen:?}");
    };
    assert_eq!(kpar_url.as_str(), "https://example.com/lib.kpar");
    assert_eq!(publisher.as_str(), "acme");
    assert_eq!(name.as_str(), "lib");
}

// ------------------------ a base per input project

/// A `.project.json` *and* a `.meta.json` at `at`, published as
/// `acme/<name>`. The solver needs only the first; locking an input project
/// without the second fails with `IncompleteProject { field: Meta }`.
#[cfg(feature = "filesystem")]
fn project_dir(at: &Utf8Path, name: &str) {
    std::fs::create_dir_all(at).expect("the project directory");
    std::fs::write(
        at.join(".project.json"),
        format!(r#"{{"name": "{name}", "publisher": "acme", "version": "1.0.0", "usage": []}}"#),
    )
    .expect("a manifest");
    std::fs::write(
        at.join(".meta.json"),
        br#"{"created": "2001-02-03T04:05:06Z", "index": {}, "checksum": {}}"#,
    )
    .expect("a metadata file");
}

#[cfg(feature = "filesystem")]
fn dir_usage(dir: &str, name: &str) -> crate::model::InterchangeProjectUsage {
    crate::model::InterchangeProjectUsage::Directory {
        dir: dir.into(),
        publisher: crate::model::ProjectPublisher::parse("acme".to_owned()).unwrap(),
        name: crate::model::ProjectName::parse(name.to_owned()).unwrap(),
    }
}

/// A context whose project root is `at`.
///
/// The base a usage resolves against and the root a solved dependency's
/// sources are relativized against are different things; without a root,
/// locking panics.
#[cfg(feature = "filesystem")]
fn ctx_rooted_at(at: &Utf8Path) -> ProjectContext {
    ProjectContext::new(
        None,
        Some(crate::project::local_src::LocalSrcProject::new_access(
            at.to_owned(),
            None,
        )),
        None,
    )
}

/// A member's `dir` is relative to that member, so two input projects do not share a base.
///
/// The two targets are *different* projects: the solver coalesces
/// usages by publisher and name, so naming one would let either base
/// answer both.
#[cfg(feature = "filesystem")]
#[test]
fn two_input_projects_resolve_against_their_own_bases() -> Result<(), Box<dyn std::error::Error>> {
    use crate::{
        commands::lock::do_lock_projects_with_bases, project::memory::InMemoryProject,
        resolve::file::FileResolver,
    };

    let tmp = camino_tempfile::tempdir()?;
    project_dir(&tmp.path().join("alpha/dep"), "one");
    project_dir(&tmp.path().join("beta/dep"), "two");

    let alpha = InMemoryProject::from_info_meta(
        serde_json::from_str(
            r#"{"name": "alpha", "publisher": "acme", "version": "1.0.0",
                "usage": [{"dir": "dep", "publisher": "acme", "name": "one"}]}"#,
        )?,
        serde_json::from_str(r#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#)?,
    );
    let beta = InMemoryProject::from_info_meta(
        serde_json::from_str(
            r#"{"name": "beta", "publisher": "acme", "version": "1.0.0",
                "usage": [{"dir": "dep", "publisher": "acme", "name": "two"}]}"#,
        )?,
        serde_json::from_str(r#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#)?,
    );

    // Wrapped editable, as a workspace's members are: an editable
    // project reports its own path as its source and never asks the
    // inner project for one.
    let alpha = crate::project::editable::EditableProject::new("alpha".into(), alpha);
    let beta = crate::project::editable::EditableProject::new("beta".into(), beta);

    let outcome = do_lock_projects_with_bases(
        vec![
            (None, &alpha, Some(tmp.path().join("alpha"))),
            (None, &beta, Some(tmp.path().join("beta"))),
        ],
        FileResolver::default(),
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;

    // Two inputs plus the two distinct targets they each resolve.
    assert_eq!(outcome.projects.len(), 4, "{:?}", outcome.projects);
    Ok(())
}

/// An input project records the editable path it was given, not `"."`.
#[cfg(feature = "filesystem")]
#[test]
fn an_input_project_records_its_own_editable_path() -> Result<(), Box<dyn std::error::Error>> {
    use crate::{
        commands::lock::do_lock_projects_with_bases, project::editable::EditableProject,
        resolve::null::NullResolver,
    };

    let member = |name: &str| -> Result<_, Box<dyn std::error::Error>> {
        Ok(InMemoryProject::from_info_meta(
            serde_json::from_str(&format!(
                r#"{{"name": "{name}", "publisher": "acme", "version": "1.0.0", "usage": []}}"#
            ))?,
            serde_json::from_str(r#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#)?,
        ))
    };
    let app = EditableProject::new("app".into(), member("app")?);
    let gamma = EditableProject::new("libs/gamma".into(), member("gamma")?);

    let outcome = do_lock_projects_with_bases(
        vec![(None, &app, None), (None, &gamma, None)],
        NullResolver {},
        &HashMap::default(),
        &ProjectContext::default(),
    )
    .map_err(|e| e.to_string())?;

    let editables: Vec<&str> = outcome
        .projects
        .iter()
        .flat_map(|p| &p.sources)
        .filter_map(|s| match s {
            Source::Editable { editable } => Some(editable.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(editables, vec!["app", "libs/gamma"]);
    Ok(())
}

/// The wrapper derives one base from the context, for every
/// usage. A `Resource` usage would not show it: only a relative `dir`
/// consults the base at all.
#[cfg(feature = "filesystem")]
#[test]
fn do_lock_projects_still_derives_one_base() -> Result<(), Box<dyn std::error::Error>> {
    use crate::{project::memory::InMemoryProject, resolve::file::FileResolver};

    let tmp = camino_tempfile::tempdir()?;
    project_dir(&tmp.path().join("dep"), "one");
    project_dir(&tmp.path().join("other"), "two");

    let app = InMemoryProject::from_info_meta(
        serde_json::from_str(
            r#"{"name": "app", "publisher": "acme", "version": "1.0.0",
                "usage": [{"dir": "dep", "publisher": "acme", "name": "one"},
                          {"dir": "other", "publisher": "acme", "name": "two"}]}"#,
        )?,
        serde_json::from_str(r#"{"created": "2001-02-03T04:05:06Z", "index": {}}"#)?,
    );

    let app = crate::project::editable::EditableProject::new("app".into(), app);

    let outcome = do_lock_projects(
        vec![(None, &app)],
        FileResolver::default(),
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(outcome.projects.len(), 3, "{:?}", outcome.projects);
    Ok(())
}

/// The same for the other wrapper, asserting the reason: with no base the
/// resolver says so.
#[cfg(feature = "filesystem")]
#[test]
fn do_lock_extend_still_derives_one_base() -> Result<(), Box<dyn std::error::Error>> {
    use crate::resolve::file::FileResolver;

    let tmp = camino_tempfile::tempdir()?;
    project_dir(&tmp.path().join("dep"), "one");

    let outcome = do_lock_extend(
        Lock::default(),
        vec![(
            dir_usage("dep", "one"),
            crate::commands::lock::DeclaredBy::Input("the test".to_owned()),
        )],
        FileResolver::default(),
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(outcome.projects.len(), 1, "{:?}", outcome.projects);
    Ok(())
}

// ------------------------- locking a workspace

/// The `file:` URL of an absolute path.
#[cfg(feature = "filesystem")]
fn file_url(path: impl AsRef<Path>) -> Url {
    Url::from_file_path(path).expect("an absolute path")
}

/// A minimal `model.toml` workspace on disk.
#[cfg(feature = "filesystem")]
fn workspace_tree(members: &[(&str, &str)], root_extra: &str) -> camino_tempfile::Utf8TempDir {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    let entries: Vec<String> = members.iter().map(|(at, _)| format!("\"{at}\"")).collect();
    std::fs::write(
        dir.path().join("model.toml"),
        format!(
            "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [{}]\n{root_extra}",
            entries.join(", ")
        ),
    )
    .expect("a root manifest");
    for (at, body) in members {
        let path = dir
            .path()
            .join(at.trim_start_matches("./").trim_end_matches('/'));
        std::fs::create_dir_all(&path).expect("a member directory");
        std::fs::write(path.join("model.toml"), *body).expect("a member manifest");
    }
    dir
}

#[cfg(feature = "filesystem")]
fn member_toml(name: &str, usages: &str) -> String {
    format!(
        "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
         version = \"1.0.0\"\nsources = []\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n{usages}"
    )
}

#[cfg(feature = "filesystem")]
#[expect(
    clippy::result_large_err,
    reason = "the lock's own error, returned as is so tests can match on it"
)]
fn lock_workspace(
    dir: &camino_tempfile::Utf8TempDir,
) -> Result<
    crate::lock::Lock,
    LockProjectError<
        crate::project::editable::EditableProject<crate::project::local_src::LocalSrcProject>,
        crate::resolve::file::FileResolverProject,
        crate::resolve::file::FileResolver,
    >,
> {
    use crate::workspace::model_toml::{MemberChecks, Workspace};
    let workspace =
        Workspace::load_with(dir.path(), MemberChecks::All).expect("the workspace loads");
    let ctx = ProjectContext::new(Some(workspace), None, None);
    super::do_lock_workspace(
        ctx.workspace().expect("just set"),
        crate::resolve::file::FileResolver::default(),
        &HashMap::default(),
        &ctx,
    )
}

/// Two members of one workspace that disagree about a dependency's version.
///
/// The `FileResolver` serves local paths only, and an intra-workspace `dir`
/// usage carries no version constraint at all, so the conflict needs an
/// index usage and a resolver that can offer two versions of it.
#[cfg(feature = "filesystem")]
fn lock_workspace_with<R>(
    dir: &camino_tempfile::Utf8TempDir,
    resolver: R,
) -> Result<
    crate::lock::Lock,
    LockProjectError<
        crate::project::editable::EditableProject<crate::project::local_src::LocalSrcProject>,
        R::ProjectStorage,
        R,
    >,
>
where
    R: ResolveRead + std::fmt::Debug,
    R::ProjectStorage: crate::project::ProjectRead + std::fmt::Debug,
{
    use crate::workspace::model_toml::{MemberChecks, Workspace};
    let workspace =
        Workspace::load_with(dir.path(), MemberChecks::All).expect("the workspace loads");
    let ctx = ProjectContext::new(Some(workspace), None, None);
    super::do_lock_workspace(
        ctx.workspace().expect("just set"),
        resolver,
        &HashMap::default(),
        &ctx,
    )
}

/// A candidate `acme/lib` at `version`, for a resolver to offer.
#[cfg(feature = "filesystem")]
fn lib_at(version: &str) -> InMemoryProject {
    let mut project = InMemoryProject::from_info_meta(
        InterchangeProjectInfoRaw {
            name: "lib".into(),
            publisher: Some("acme".into()),
            version: version.into(),
            description: None,
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        },
        InterchangeProjectMetadataRaw {
            index: Some(IndexMap::default()),
            root: None,
            exports: None,
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

/// A workspace is solved as one root request, so every member's usage
/// is in the same coalesced package; the conflict still names the
/// member behind each side.
#[cfg(feature = "filesystem")]
#[test]
fn a_workspace_lock_names_the_members_that_disagree() {
    use crate::project::utils::Identifier;
    use crate::resolve::memory::{AcceptAll, MemoryResolver};

    let dir = workspace_tree(
        &[
            (
                "app",
                &member_toml(
                    "app",
                    "[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^1\"\n",
                ),
            ),
            (
                "tool",
                &member_toml(
                    "tool",
                    "[[inline-usages]]\npublisher = \"acme\"\nname = \"lib\"\nversion = \"^2\"\n",
                ),
            ),
        ],
        "",
    );

    let resolver = MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: [(
            Identifier::from_iri_unchecked_str("pkg:sysand/acme/lib"),
            vec![lib_at("1.0.0"), lib_at("2.0.0")],
        )]
        .into(),
    };

    let error = lock_workspace_with(&dir, resolver)
        .expect_err("two members cannot agree on one version of `lib`");
    let msg = error.to_string();

    assert!(
        msg.contains("`pkg:sysand/acme/app` requires `pkg:sysand/acme/lib` at `^1`"),
        "got: {msg}"
    );
    assert!(
        msg.contains("`pkg:sysand/acme/tool` requires `pkg:sysand/acme/lib` at `^2`"),
        "got: {msg}"
    );
    assert!(
        msg.contains("only one version of each dependency can be used"),
        "got: {msg}"
    );
}

#[cfg(feature = "filesystem")]
fn editables(lock: &crate::lock::Lock) -> Vec<&str> {
    let mut paths: Vec<&str> = lock
        .projects
        .iter()
        .flat_map(|p| &p.sources)
        .filter_map(|s| match s {
            Source::Editable { editable } => Some(editable.as_str()),
            _ => None,
        })
        .collect();
    paths.sort_unstable();
    paths
}

/// Every member is in the lockfile, recorded as editable at its own
/// path.
#[cfg(feature = "filesystem")]
#[test]
fn a_workspace_locks_every_member() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[
            ("app", &member_toml("alpha", "")),
            ("libs/gamma", &member_toml("gamma", "")),
        ],
        "",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(editables(&lock), vec!["app", "libs/gamma"]);
    Ok(())
}

#[cfg(feature = "filesystem")]
#[test]
fn a_member_usage_of_another_member_resolves() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[
            ("app", &member_toml("alpha", "")),
            (
                "beta",
                &member_toml(
                    "beta",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"alpha\"\ndir = \"../app\"\n",
                ),
            ),
        ],
        "",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(
        lock.projects.iter().filter(|p| p.name == "alpha").count(),
        1,
        "`alpha` is one project, not one per way of naming it: {:?}",
        lock.projects
    );
    Ok(())
}

/// The base a member's `dir` resolves against is that member's own
/// directory. With every member at depth 1 this proves nothing, so
/// `libs/gamma` reaches back two levels.
#[cfg(feature = "filesystem")]
#[test]
fn members_at_different_depths_resolve_their_own_bases() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[
            ("app", &member_toml("alpha", "")),
            (
                "libs/gamma",
                &member_toml(
                    "gamma",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"alpha\"\ndir = \"../../app\"\n",
                ),
            ),
        ],
        "",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(editables(&lock), vec!["app", "libs/gamma"]);
    Ok(())
}

/// A group entry's paths are relative to the workspace root, and the
/// view drops that distinction, so it is refused.
#[cfg(feature = "filesystem")]
#[test]
fn an_inherited_local_usage_is_refused() {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"shared\"\nworkspace = \"legacy\"\n",
            ),
        )],
        "\n[workspace.group.legacy]\n\n[[workspace.group.legacy.inline-usages]]\n\
         publisher = \"acme\"\nname = \"shared\"\ndir = \"vendor/shared\"\n",
    );
    let error = lock_workspace(&dir).expect_err("an inherited local usage");
    assert_matches!(
        error,
        LockProjectError::InheritedLocalUsage {
            member: _,
            usage: _,
            path: _
        },
        "{error}"
    );
    let text = error.to_string();
    assert!(text.contains("app"), "{text}");
    assert!(text.contains("vendor/shared"), "{text}");
}

/// And only a *local* one: an inherited index usage is legitimate.
#[cfg(feature = "filesystem")]
#[test]
fn an_inherited_index_usage_is_not_refused() {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"shared\"\nworkspace = \"legacy\"\n",
            ),
        )],
        "\n[workspace.group.legacy]\n\n[[workspace.group.legacy.inline-usages]]\n\
         publisher = \"acme\"\nname = \"shared\"\nversion = \"^1\"\n",
    );
    let error = lock_workspace(&dir).expect_err("`acme/shared` resolves nowhere here");
    assert!(
        !matches!(
            error,
            LockProjectError::InheritedLocalUsage {
                member: _,
                usage: _,
                path: _
            }
        ),
        "an index usage must not be refused as a local one: {error}"
    );
}

#[cfg(feature = "filesystem")]
#[test]
fn members_are_identified_by_their_purls() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(&[("app", &member_toml("alpha", ""))], "");
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(
        lock.projects[0].identifiers,
        vec!["pkg:sysand/acme/alpha".to_owned()]
    );
    Ok(())
}

/// A solved local dependency's `src_path` is recorded against the *workspace* root,
/// not against the member whose usage reached it. From `app`, the
/// target is `../vendor/dep`; in the lockfile it is `vendor/dep`.
#[cfg(feature = "filesystem")]
#[test]
fn a_dependency_path_is_relative_to_the_workspace_root() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../vendor/dep\"\n",
            ),
        )],
        // `exclude`d, because a `dir` usage of a project under the root in
        // neither list is the unlisted-project error, and this test is about
        // the shape of the recorded path.
        "exclude = [\"vendor\"]\n",
    );
    project_dir(&dir.path().join("vendor/dep"), "dep");

    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    let dep_sources: Vec<&Source> = lock
        .projects
        .iter()
        .filter(|p| p.name == "dep")
        .flat_map(|p| &p.sources)
        .collect();
    assert_eq!(
        dep_sources,
        [&Source::Editable {
            editable: "vendor/dep".into()
        }]
    );
    Ok(())
}

/// A member that inherits and is actually locked, so the table reaches
/// `new_member`.
#[cfg(feature = "filesystem")]
#[test]
fn an_inheriting_member_locks_with_the_tables_value() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version.workspace = true\nsources = []\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
        )],
        "version = \"9.9.9\"\n",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(lock.projects[0].version, "9.9.9");
    Ok(())
}

/// A `dir` usage from a member to a project that lies under the root
/// without being a member (and not covered in the `exclude` list) is the
/// unlisted-project error.
/// The rule applies through a usage as well as on the upward walk from
/// the stray project's own directory.
#[cfg(feature = "filesystem")]
#[test]
fn a_dir_usage_of_an_unlisted_project_is_refused() {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"stray\"\ndir = \"../stray\"\n",
            ),
        )],
        "",
    );
    // Under the root, in neither list.
    std::fs::create_dir_all(dir.path().join("stray")).expect("the stray directory");
    std::fs::write(
        dir.path().join("stray/model.toml"),
        member_toml("stray", ""),
    )
    .expect("the stray manifest");

    let error = lock_workspace(&dir).expect_err("an unlisted target is the unlisted-project error");
    // With its chain of causes, as the CLI prints it: the solver names the
    // failing project and the cause says why.
    let message = crate::utils::format_err(error);
    assert!(
        message.contains("is neither listed in `workspace.members` nor under `workspace.exclude`"),
        "got: {message}"
    );
}

/// The unlisted-project rule, reached through the resolver's `Resource`
/// arm.
///
/// Only a **json** project's legacy `resource` usage reaches it with a
/// `file:` URL, and it keeps both candidates.
///
/// The topology: an `exclude`d json project reached by a `dir` usage from a
/// member.
#[cfg(feature = "filesystem")]
#[test]
fn a_file_url_usage_of_an_unlisted_project_is_refused() {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"ext\"\ndir = \"../ext\"\n",
            ),
        )],
        "exclude = [\"ext\"]\n",
    );
    std::fs::create_dir_all(dir.path().join("stray")).expect("the stray directory");
    std::fs::write(
        dir.path().join("stray/model.toml"),
        member_toml("stray", ""),
    )
    .expect("the stray manifest");
    // The `file:` URL is absolute, so it is written once the tree exists.
    std::fs::create_dir_all(dir.path().join("ext")).expect("the excluded directory");
    std::fs::write(
        dir.path().join("ext/.project.json"),
        format!(
            r#"{{"name": "ext", "publisher": "acme", "version": "1.0.0",
                "usage": [{{"resource": "{}"}}]}}"#,
            file_url(dir.path().join("stray"))
        ),
    )
    .expect("the excluded manifest");
    std::fs::write(
        dir.path().join("ext/.meta.json"),
        "{\"index\": {}, \"created\": \"2025-05-30T12:34:24.977672Z\"}",
    )
    .expect("the excluded metadata");

    let error = lock_workspace(&dir)
        .expect_err("an unlisted target is refused however the usage is spelled");
    let message = crate::utils::format_err(error);
    assert!(
        message.contains("is neither listed in `workspace.members` nor under `workspace.exclude`"),
        "got: {message}"
    );
}

/// A **json** project's legacy `resource` usage names a directory or an
/// archive.
#[cfg(feature = "filesystem")]
#[test]
fn a_json_resource_naming_a_directory_still_resolves() {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(dir.path().join("dep")).expect("the target directory");
    std::fs::write(dir.path().join("dep/model.toml"), member_toml("dep", ""))
        .expect("the target manifest");
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\nexclude = [\"dep\", \"ext\"]\n",
    )
    .expect("the root manifest");
    std::fs::create_dir_all(dir.path().join("app")).expect("the member directory");
    std::fs::write(
        dir.path().join("app/model.toml"),
        member_toml(
            "alpha",
            "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"ext\"\ndir = \"../ext\"\n",
        ),
    )
    .expect("the member manifest");
    std::fs::create_dir_all(dir.path().join("ext")).expect("the json directory");
    std::fs::write(
        dir.path().join("ext/.project.json"),
        format!(
            r#"{{"name": "ext", "publisher": "acme", "version": "1.0.0",
                "usage": [{{"resource": "{}"}}]}}"#,
            file_url(dir.path().join("dep"))
        ),
    )
    .expect("the json manifest");
    std::fs::write(
        dir.path().join("ext/.meta.json"),
        "{\"index\": {}, \"created\": \"2025-05-30T12:34:24.977672Z\"}",
    )
    .expect("the json metadata");

    lock_workspace(&dir).expect("a json `resource` usage still names a directory");
}

/// A workspace member's `kpar-url` usage whose name differs from its
/// target's is refused.
#[cfg(feature = "filesystem")]
#[test]
fn a_member_kpar_url_that_misnames_its_target_is_refused() {
    const REMOTE: &str = "https://example.com/dep.kpar";
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        dir.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n[workspace]\nmembers = [\"app\"]\n",
    )
    .expect("the root manifest");
    std::fs::create_dir_all(dir.path().join("app")).expect("the member directory");
    std::fs::write(
        dir.path().join("app/model.toml"),
        member_toml(
            "alpha",
            &format!(
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"wrongname\"\n\
                 kpar-url = \"{REMOTE}\"\n"
            ),
        ),
    )
    .expect("the member manifest");

    let mut info = InterchangeProjectInfoRaw::minimal("dep".to_owned(), "1.0.0".to_owned());
    info.publisher = Some("acme".to_owned());
    let mut target = InMemoryProject::from_info_meta(
        info,
        InterchangeProjectMetadataRaw::minimal("1970-01-01T00:00:00Z".to_owned()),
    );
    target.nominal_sources = vec![Source::RemoteKpar {
        remote_kpar: fluent_uri::Iri::parse(REMOTE.to_owned()).expect("a valid IRI"),
        kpar_size: std::num::NonZeroU64::MIN,
        kpar_digest: "0".repeat(64),
    }];
    let resolver = crate::resolve::memory::MemoryResolver::resources_only([(
        crate::project::utils::Identifier::from_iri_owned(
            fluent_uri::Iri::parse(REMOTE.to_owned()).expect("a valid IRI"),
        ),
        vec![target],
    )]);
    let error =
        lock_workspace_with(&dir, resolver).expect_err("a member's usage names its target too");
    let message = crate::utils::format_err(error);
    assert!(
        message.contains("is rejected because its spelling does not match the project's"),
        "got: {message}"
    );
}

// ------------------- one identity named at two places

/// Two members whose `dir` usages name **different** directories that
/// declare one identity.
#[cfg(feature = "filesystem")]
#[test]
fn one_identity_at_two_directories_is_refused() {
    let dir = workspace_tree(
        &[
            (
                "one",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v1\"\n",
                ),
            ),
            (
                "two",
                &member_toml(
                    "beta",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v2\"\n",
                ),
            ),
        ],
        "exclude = [\"deps\"]\n",
    );
    for at in ["deps/v1", "deps/v2"] {
        std::fs::create_dir_all(dir.path().join(at)).expect("the target directory");
        std::fs::write(
            dir.path().join(at).join("model.toml"),
            member_toml("dep", ""),
        )
        .expect("the target manifest");
    }

    let error = lock_workspace(&dir).expect_err("one identity cannot come from two places");
    let message = crate::utils::format_err(error);
    assert!(
        message.contains("is named at two places"),
        "and it names both: {message}"
    );
    assert!(
        message.contains(Utf8Path::new("deps").join("v1").as_str())
            && message.contains(Utf8Path::new("deps").join("v2").as_str()),
        "both paths are in the message: {message}"
    );
}

/// The same collision reached through a `kpar-path` on one side.
///
/// The cache keys on identity alone, so the two usages need not be the
/// same *kind*: a `dir` meeting a `kpar-path` collides exactly as two
/// `dir`s do.
#[cfg(feature = "filesystem")]
#[test]
fn one_identity_at_a_directory_and_an_archive_is_refused() {
    let dir = workspace_tree(
        &[
            (
                "one",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v1\"\n",
                ),
            ),
            (
                "two",
                &member_toml(
                    "beta",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\n\
                     kpar-path = \"../deps/dep.kpar\"\n",
                ),
            ),
        ],
        "exclude = [\"deps\"]\n",
    );
    std::fs::create_dir_all(dir.path().join("deps/v1")).expect("the target directory");
    std::fs::write(
        dir.path().join("deps/v1/model.toml"),
        member_toml("dep", ""),
    )
    .expect("the target manifest");
    crate::commands::build::do_build_kpar(
        &crate::project::local_src::LocalSrcProject::new_access(dir.path().join("deps/v1"), None),
        dir.path().join("deps/dep.kpar"),
        crate::commands::build::KparCompressionMethod::Stored,
        true,
        false,
    )
    .expect("the archive builds");

    let error = lock_workspace(&dir).expect_err("a directory and an archive are still two places");
    let message = crate::utils::format_err(error);
    assert!(message.contains("is named at two places"), "got: {message}");
}

/// The guard against refusing the ordinary case: two members depending
/// on **one** directory is what unifying by identity is for.
#[cfg(feature = "filesystem")]
#[test]
fn one_identity_at_one_directory_still_locks() {
    let dir = workspace_tree(
        &[
            (
                "one",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v1\"\n",
                ),
            ),
            (
                "two",
                &member_toml(
                    "beta",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v1\"\n",
                ),
            ),
        ],
        "exclude = [\"deps\"]\n",
    );
    std::fs::create_dir_all(dir.path().join("deps/v1")).expect("the target directory");
    std::fs::write(
        dir.path().join("deps/v1/model.toml"),
        member_toml("dep", ""),
    )
    .expect("the target manifest");

    lock_workspace(&dir).expect("one place is not a collision");
}

/// And the same directory spelled differently from two depths.
#[cfg(feature = "filesystem")]
#[test]
fn one_directory_spelled_two_ways_still_locks() {
    let dir = workspace_tree(
        &[
            (
                "one",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../deps/v1\"\n",
                ),
            ),
            (
                "nest/two",
                &member_toml(
                    "beta",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\n\
                     dir = \"../../deps/v1\"\n",
                ),
            ),
        ],
        "exclude = [\"deps\"]\n",
    );
    std::fs::create_dir_all(dir.path().join("deps/v1")).expect("the target directory");
    std::fs::write(
        dir.path().join("deps/v1/model.toml"),
        member_toml("dep", ""),
    )
    .expect("the target manifest");

    lock_workspace(&dir).expect("two spellings of one directory are one place");
}

/// A metamodel direction error stops the lock.
///
/// The edge checks run once **before the lockfile is written** and
/// again when the lockfile is validated.
#[cfg(feature = "filesystem")]
#[test]
fn a_direction_error_fails_the_lock() {
    let dir = workspace_tree(
        &[
            (
                "app",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"beta\"\ndir = \"../lib\"\n",
                ),
            ),
            (
                "lib",
                "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"beta\"\n\
                 version = \"1.0.0\"\nsources = [\"main.sysml\"]\n\
                 metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"main.sysml\"\n",
            ),
        ],
        "",
    );
    std::fs::write(dir.path().join("lib/main.sysml"), "package Beta;\n").expect("a source file");

    let error = lock_workspace(&dir).expect_err("an OMG project may not depend on a Syside one");
    let message = error.to_string();
    assert!(
        message.contains("only Syside-metamodel projects may depend on Syside-metamodel projects"),
        "got: {message}"
    );
    assert!(
        message.contains("acme/alpha") && message.contains("acme/beta"),
        "both projects must be named: {message}"
    );
}

/// A member's `dir` usage of another member resolves even when the
/// target inherits a key from the table.
///
/// Other tests lock a member-to-member `dir` usage with a literal
/// `metamodel` in every member. Here the target *inherits*, and a manifest
/// with an inheritance marker cannot be read without the table, so the
/// solver's read of the target has to carry it.
#[cfg(feature = "filesystem")]
#[test]
fn a_dir_usage_of_an_inheriting_member_resolves() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[
            (
                "app",
                &member_toml(
                    "alpha",
                    "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"beta\"\ndir = \"../lib\"\n",
                ),
            ),
            (
                "lib",
                "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"beta\"\n\
                 version = \"1.0.0\"\nsources = []\nmetamodel.workspace = \"fam\"\n",
            ),
        ],
        "[workspace.group.fam]\nmetamodel = \"https://www.omg.org/spec/SysML/20250201\"\n",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(editables(&lock), vec!["app", "lib"]);
    Ok(())
}

/// The *input* project's own entry records its metamodel, not only its
/// dependencies'.
#[cfg(feature = "filesystem")]
#[test]
fn a_members_own_entry_records_its_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version = \"1.0.0\"\nsources = [\"main.sysml\"]\n\
             metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"main.sysml\"\n",
        )],
        "",
    );
    std::fs::write(dir.path().join("app/main.sysml"), "package Alpha;\n")?;
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    let member = lock
        .projects
        .iter()
        .find(|p| p.name == "alpha")
        .expect("the member is in the lockfile");
    assert_eq!(
        member.metamodel.as_deref(),
        Some("https://www.sysand.com/spec/SysML/20250201")
    );
    Ok(())
}

/// A Syside dependency's `exports` come from its `.meta.json`'s
/// `exports` array, and a Syside member's from the symbols of its
/// sources, the same input to the collision check as an OMG project's
/// `index` keys.
#[cfg(feature = "filesystem")]
#[test]
fn a_syside_json_dependency_locks_with_its_exports() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version = \"1.0.0\"\nsources = [\"main.sysml\"]\n\
             metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"main.sysml\"\n\
             \n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../dep\"\n",
        )],
        // The dependency sits under the workspace root, so it is
        // excluded rather than a stray.
        "exclude = [\"dep\"]\n",
    );
    std::fs::write(dir.path().join("app/main.sysml"), "package Alpha;\n")?;
    let dep = dir.path().join("dep");
    std::fs::create_dir_all(&dep)?;
    std::fs::write(
        dep.join(".project.json"),
        r#"{"name": "dep", "publisher": "acme", "version": "1.0.0", "usage": []}"#,
    )?;
    std::fs::write(
        dep.join(".meta.json"),
        r#"{"root": "lib.sysml", "exports": ["A", "B"], "created": "2001-02-03T04:05:06Z",
            "metamodel": "https://www.sysand.com/spec/SysML/20250201",
            "checksum": {"lib.sysml": {"value": "0000000000000000000000000000000000000000000000000000000000000000", "algorithm": "SHA256"}}}"#,
    )?;
    std::fs::write(dep.join("lib.sysml"), "package A; package B;\n")?;

    let lock = lock_workspace(&dir).map_err(crate::utils::format_err)?;
    let dep = lock
        .projects
        .iter()
        .find(|p| p.name == "dep")
        .expect("the dependency is in the lockfile");
    assert_eq!(dep.exports, ["A".to_owned(), "B".to_owned()]);
    let app = lock
        .projects
        .iter()
        .find(|p| p.name == "alpha")
        .expect("the member is in the lockfile");
    assert_eq!(app.exports, ["Alpha".to_owned()]);
    Ok(())
}

/// A Syside archive is a usable `kpar-path` target, and the lockfile takes its `exports` from
/// the archive's array.
#[cfg(feature = "filesystem")]
#[test]
fn a_syside_kpar_is_a_usable_kpar_path_target() -> Result<(), Box<dyn std::error::Error>> {
    use crate::commands::build::{KparCompressionMethod, do_build_kpar};
    use crate::project::local_src::LocalSrcProject;

    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version = \"1.0.0\"\nsources = [\"main.sysml\"]\n\
             metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"main.sysml\"\n\
             \n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\nkpar-path = \"../dep.kpar\"\n",
        )],
        "",
    );
    std::fs::write(dir.path().join("app/main.sysml"), "package Alpha;\n")?;

    let target = camino_tempfile::tempdir()?;
    std::fs::write(
        target.path().join("model.toml"),
        "manifest-version = \"1.0\"\n\n[project]\npublisher = \"acme\"\nname = \"dep\"\n\
         version = \"2.0.0\"\nsources = [\"lib.sysml\"]\n\
         metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\nroot = \"lib.sysml\"\n",
    )?;
    std::fs::write(target.path().join("lib.sysml"), "package A; package B;\n")?;
    do_build_kpar(
        &LocalSrcProject::new_access(target.path(), None),
        dir.path().join("dep.kpar"),
        KparCompressionMethod::default(),
        true,
        false,
    )?;

    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    let dep = lock
        .projects
        .iter()
        .find(|p| p.name == "dep")
        .expect("the archive is in the lockfile");
    assert_eq!(dep.exports, ["A".to_owned(), "B".to_owned()]);
    assert_eq!(dep.version.clone(), "2.0.0");
    Ok(())
}
