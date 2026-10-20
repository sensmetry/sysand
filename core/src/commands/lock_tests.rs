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
use crate::commands::lock::LockProjectError;
#[cfg(feature = "filesystem")]
use crate::resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead, UsageOrigin};

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
            Ok(outcome) => Ok(outcome.lock),
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
            &root(index("Acme Labs", "My Lib")),
            vec![("acme labs", "my lib", lib)],
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "an index usage `Acme Labs/My Lib` in `app` 1.0.0 resolved to version 1.0.0 of \
             `My Lib`, which declares no publisher, so no typed usage can name it;\n\
             use a resource usage of it instead (e.g. `sysand add --iri <IRI>`)"
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

/// A resolver that records the origin of every usage it is asked to
/// resolve, and resolves nothing: the solve fails after the first
/// usage, which is enough to see what was asked.
#[cfg(feature = "filesystem")]
#[derive(Debug, Default)]
struct RecordingResolver {
    seen: std::rc::Rc<std::cell::RefCell<Vec<UsageOrigin>>>,
}

#[cfg(feature = "filesystem")]
impl ResolveRead for RecordingResolver {
    type Error = std::convert::Infallible;

    type ProjectStorage = crate::project::null::NullProject;

    type ResolvedStorages = std::iter::Empty<Result<Self::ProjectStorage, Self::Error>>;

    fn resolve_read(
        &self,
        resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        self.seen.borrow_mut().push(resolve.origin());
        Ok(ResolutionOutcome::NotFound {
            reason: "recording resolver".to_owned(),
        })
    }
}

/// The usages of the projects handed to `do_lock_projects` are the
/// solve's *requested* usages, and their origin is the format of the
/// project that declared them -- there is no candidate to ask.
#[cfg(feature = "filesystem")]
#[test]
fn a_requested_usage_takes_the_origin_of_the_project_that_declared_it() {
    use crate::project::local_src::LocalSrcProject;

    fn origins_of(dir: &camino::Utf8Path) -> Vec<UsageOrigin> {
        let project =
            LocalSrcProject::new_access(dir, Some(typed_path::Utf8UnixPathBuf::from(".")));
        let resolver = RecordingResolver::default();
        let seen = resolver.seen.clone();
        do_lock_projects(
            [(None, &project)],
            resolver,
            &HashMap::new(),
            &ProjectContext::default(),
        )
        .expect_err("the resolver finds nothing");
        seen.borrow().clone()
    }

    let toml = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        toml.path().join("model.toml"),
        "manifest-version = 1\n\n\
         [project]\n\
         publisher = \"acme\"\n\
         name = \"app\"\n\
         version = \"1.0.0\"\n\
         sources = []\n\n\
         [[inline-usages]]\n\
         publisher = \"acme\"\n\
         name = \"lib\"\n\
         version = \"^1\"\n",
    )
    .expect("write model.toml");
    assert_eq!(origins_of(toml.path()), [UsageOrigin::Toml]);

    let json = camino_tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        json.path().join(".project.json"),
        r#"{"name":"app","version":"1.0.0","usage":[{"resource":"pkg:sysand/acme/lib"}]}"#,
    )
    .expect("write .project.json");
    std::fs::write(
        json.path().join(".meta.json"),
        r#"{"index":{},"created":"1970-01-01T00:00:00Z"}"#,
    )
    .expect("write .meta.json");
    assert_eq!(origins_of(json.path()), [UsageOrigin::Json]);
}

// ------------------------ a base per input project

/// A `.project.json` *and* a `.meta.json` at `at`, published as
/// `acme/<name>`. The solver needs only the first; `do_lock_extend`
/// returns `IncompleteProject { field: Meta }` without the second.
#[cfg(feature = "filesystem")]
fn project_dir(at: &camino::Utf8Path, name: &str) {
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
/// Every test here needs one. The base a usage resolves against and the
/// root a solved dependency's sources are relativized against are
/// different things, and without a root, `LocalSrcProject::sources`
/// panics.
#[cfg(feature = "filesystem")]
fn ctx_rooted_at(at: &camino::Utf8Path) -> ProjectContext {
    ProjectContext {
        current_project: Some(crate::project::local_src::LocalSrcProject::new_access(
            at.to_owned(),
            None,
        )),
        ..Default::default()
    }
}

/// `workspaces.md` "Intra-workspace usages": a member's `dir` is
/// relative to that member, so two input projects do not share a base.
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
        FileResolver {
            sandbox_roots: None,
        },
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;

    // Two inputs plus the two distinct targets they each resolve.
    assert_eq!(
        outcome.lock.projects.len(),
        4,
        "{:?}",
        outcome.lock.projects
    );
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
        .lock
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

/// The wrapper still derives one base from the context, for every
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
        FileResolver {
            sandbox_roots: None,
        },
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(
        outcome.lock.projects.len(),
        3,
        "{:?}",
        outcome.lock.projects
    );
    Ok(())
}

/// The same for the other wrapper. Asserting the reason: with no base
/// the resolver says so, and `is_err` alone would not tell that apart
/// from any other resolution failure.
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
            UsageOrigin::Json,
        )],
        FileResolver {
            sandbox_roots: None,
        },
        &HashMap::default(),
        &ctx_rooted_at(tmp.path()),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(
        outcome.lock.projects.len(),
        1,
        "{:?}",
        outcome.lock.projects
    );
    Ok(())
}

// ------------------------- locking a workspace

/// A `model.toml` workspace on disk. `Fixture` in `model_toml_tests.rs`
/// is a private test module, so this is its minimum here.
#[cfg(feature = "filesystem")]
fn workspace_tree(members: &[(&str, &str)], root_extra: &str) -> camino_tempfile::Utf8TempDir {
    let dir = camino_tempfile::tempdir().expect("a temporary directory");
    let entries: Vec<String> = members.iter().map(|(at, _)| format!("\"{at}\"")).collect();
    std::fs::write(
        dir.path().join("model.toml"),
        format!(
            "manifest-version = 1\n\n[workspace]\nmembers = [{}]\n{root_extra}",
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
        "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"{name}\"\n\
         version = \"1.0.0\"\nsources = []\n{usages}"
    )
}

/// The context `do_lock_workspace` requires: its sources root is the
/// workspace root, canonicalized as `load_with` reports it.
#[cfg(feature = "filesystem")]
fn workspace_ctx(root: &camino::Utf8Path) -> ProjectContext {
    ProjectContext {
        model_workspace_root: Some(root.to_owned()),
        ..Default::default()
    }
}

#[cfg(feature = "filesystem")]
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
    let workspace = Workspace::load_with(dir.path(), MemberChecks::SkipStrayState)
        .expect("the workspace loads");
    let ctx = workspace_ctx(workspace.root());
    super::do_lock_workspace(
        &workspace,
        crate::resolve::file::FileResolver {
            sandbox_roots: None,
        },
        &HashMap::default(),
        &ctx,
    )
    .map(|outcome| outcome.lock)
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
/// view drops that distinction -- so it is refused rather than joined
/// onto the member.
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
    assert!(
        matches!(
            error,
            LockProjectError::InheritedLocalUsage {
                member: _,
                usage: _,
                path: _
            }
        ),
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
fn a_dependency_src_path_is_relative_to_the_workspace_root()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            &member_toml(
                "alpha",
                "\n[[inline-usages]]\npublisher = \"acme\"\nname = \"dep\"\ndir = \"../vendor/dep\"\n",
            ),
        )],
        "",
    );
    project_dir(&dir.path().join("vendor/dep"), "dep");

    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    let src_paths: Vec<&str> = lock
        .projects
        .iter()
        .flat_map(|p| &p.sources)
        .filter_map(|s| match s {
            Source::LocalSrc {
                src_path,
                checksum: _,
            } => Some(src_path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(src_paths, vec!["vendor/dep"]);
    Ok(())
}

/// A member that inherits and is actually locked. The refusal tests
/// above return before any project is built, so without this nothing
/// proves the table reaches `new_member`.
#[cfg(feature = "filesystem")]
#[test]
fn an_inheriting_member_locks_with_the_tables_value() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version.workspace = true\nsources = []\n",
        )],
        "version = \"9.9.9\"\n",
    );
    let lock = lock_workspace(&dir).map_err(|e| e.to_string())?;
    assert_eq!(lock.projects[0].version, "9.9.9");
    Ok(())
}

/// The *input* project's own entry records its metamodel, not only its
/// dependencies'.
///
/// Without it every root reads as an OMG project, and a Syside root with
/// a Syside dependency would be a false direction error -- the very
/// check this field feeds.
#[cfg(feature = "filesystem")]
#[test]
fn a_members_own_entry_records_its_metamodel() -> Result<(), Box<dyn std::error::Error>> {
    let dir = workspace_tree(
        &[(
            "app",
            "manifest-version = 1\n\n[project]\npublisher = \"acme\"\nname = \"alpha\"\n\
             version = \"1.0.0\"\nsources = []\n\
             metamodel = \"https://www.sysand.com/spec/SysML/20250201\"\n",
        )],
        "",
    );
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
