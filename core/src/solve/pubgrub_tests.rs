// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use core::slice;
use std::{collections::HashMap, fmt::Debug};

use fluent_uri::Iri;
use indexmap::IndexMap;
use semver::VersionReq;

use crate::{
    env::memory::MemoryStorageEnvironment,
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage,
        InterchangeProjectUsageRaw,
    },
    project::{ProjectRead, memory::InMemoryProject, utils::Identifier},
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead,
        env::EnvResolver,
        memory::{AcceptAll, IRIPredicate, MemoryResolver},
        sequential::SequentialResolver,
    },
};

fn trivial_memory_project<'a>(
    name: &str,
    version: &str,
    usage: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> InMemoryProject {
    memory_project(
        name,
        version,
        usage
            .into_iter()
            .map(|(d, dv)| InterchangeProjectUsageRaw::Resource {
                resource: d.to_owned(),
                version_constraint: dv.map(std::borrow::ToOwned::to_owned),
            })
            .collect(),
    )
}

fn memory_project(
    name: &str,
    version: &str,
    usage: Vec<InterchangeProjectUsageRaw>,
) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: name.to_owned(),
            publisher: None,
            description: None,
            version: version.to_owned(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage,
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: "123".to_owned(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: Some(IndexMap::default()),
        }),
        files: HashMap::default(),
        nominal_sources: vec![],
    }
}

fn memory_resolver(
    structure: &[(&str, &[InMemoryProject])],
) -> MemoryResolver<AcceptAll, InMemoryProject> {
    MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: structure
            .iter()
            .map(|(id, projs)| (Identifier::from_iri_unchecked_str(id), projs.to_vec()))
            .collect(),
    }
}

/// Which usage forms a storage can serve: a local-path-like storage serves
/// directory usages, a remote-index-like storage serves resource IRIs
#[derive(Debug)]
enum Serves {
    DirectoryUsages,
    ResourceUsages,
}

impl IRIPredicate for Serves {
    fn accept(&self, usage: &ResolutionInfo) -> bool {
        #[expect(clippy::match_like_matches_macro)]
        match (self, usage.usage()) {
            (Self::DirectoryUsages, InterchangeProjectUsage::Directory { .. }) => true,
            (Self::ResourceUsages, InterchangeProjectUsage::Resource { .. }) => true,
            _ => false,
        }
    }
}

fn memory_resolver_serving(
    serves: Serves,
    structure: &[(&str, &[InMemoryProject])],
) -> MemoryResolver<Serves, InMemoryProject> {
    MemoryResolver {
        iri_predicate: serves,
        projects: structure
            .iter()
            .map(|(id, projs)| (Identifier::from_iri_unchecked_str(id), projs.to_vec()))
            .collect(),
    }
}

/// The `(name, version)` pairs of all projects in a solution
fn solution_projects<P: ProjectRead>(solution: &HashMap<Identifier, P>) -> Vec<(String, String)> {
    solution
        .values()
        .map(|p| {
            let info = p.get_info().unwrap().unwrap();
            (info.name, info.version)
        })
        .collect()
}

fn simple_resolver_environment(
    structure: &[(&str, &[InMemoryProject])],
) -> EnvResolver<MemoryStorageEnvironment<InMemoryProject>> {
    EnvResolver {
        env: MemoryStorageEnvironment {
            projects: structure
                .iter()
                .map(|(x, projs)| {
                    (
                        x.to_string(),
                        projs
                            .iter()
                            .map(|proj| (proj.info.clone().unwrap().version, proj.clone()))
                            .collect(),
                    )
                })
                .collect(),
        },
    }
}

#[test]
fn trivial_resolution() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = simple_resolver_environment(&[]);

    let solution = super::solve(vec![], None, resolver)?;

    assert!(solution.is_empty());

    Ok(())
}

#[test]
fn version_selection() -> Result<(), Box<dyn std::error::Error>> {
    let project_v1 = trivial_memory_project("version_selection", "1.0.1", vec![]);
    let project_v2 = trivial_memory_project("version_selection", "2.0.1", vec![]);

    let resolver =
        simple_resolver_environment(&[("urn:kpar:version_selection", &[project_v1, project_v2])]);

    let solution = super::solve(
        vec![InterchangeProjectUsage::Resource {
            resource: Iri::parse("urn:kpar:version_selection")?.into(),
            version_constraint: Some(VersionReq::parse(">=2.0.0")?),
        }],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 1);

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:version_selection")];

    assert_eq!(install.version()?.unwrap(), "2.0.1");

    Ok(())
}

#[test]
fn version_constraint_default() -> Result<(), Box<dyn std::error::Error>> {
    // `semver` by default prepends `^` if a version requirement does not
    // have a comparator. This is not documented, but is also extremely
    // unlikely to change, as it's the behavior relied on by cargo
    let v_no_caret = VersionReq::parse("2.0.0")?;
    let v_caret = VersionReq::parse("^2.0.0")?;
    assert_eq!(v_no_caret, v_caret);

    Ok(())
}

/// A directory usage is resolved in an environment by the identifier
/// derived from its publisher and name; the path is not relevant
#[test]
fn directory_usage_env_single_version() -> Result<(), Box<dyn std::error::Error>> {
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);

    let resolver = simple_resolver_environment(&[("pkg:sysand/acme/widget", &[widget])]);

    let solution = super::solve(
        vec![InterchangeProjectUsage::Directory {
            dir: "some/dir".into(),
            publisher: "acme".to_owned(),
            name: "widget".to_owned(),
        }],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 1);

    let install = &solution[&Identifier::from_pub_name("acme", "widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// A directory usage carries no version constraint, so when the environment
/// contains several versions of the project, the highest one is selected
#[test]
fn directory_usage_env_multiple_versions_selects_highest() -> Result<(), Box<dyn std::error::Error>>
{
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);

    let resolver =
        simple_resolver_environment(&[("pkg:sysand/acme/widget", &[widget_v1, widget_v2])]);

    let solution = super::solve(
        vec![InterchangeProjectUsage::Directory {
            dir: "some/dir".into(),
            publisher: "acme".to_owned(),
            name: "widget".to_owned(),
        }],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 1);

    let install = &solution[&Identifier::from_pub_name("acme", "widget")];
    assert_eq!(install.version()?.unwrap(), "2.0.0");

    Ok(())
}

/// A project installed in an environment can itself have a directory usage
/// (e.g. `resolve_dependencies` encounters these when enumerating sources);
/// it is resolved in the environment like any other usage
#[test]
fn directory_usage_env_transitive() -> Result<(), Box<dyn std::error::Error>> {
    let app = memory_project(
        "app",
        "1.0.0",
        vec![InterchangeProjectUsageRaw::Directory {
            dir: "../widget".to_owned(),
            publisher: "acme".to_owned(),
            name: "widget".to_owned(),
        }],
    );
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);

    let resolver = simple_resolver_environment(&[
        ("pkg:sysand/acme/app", &[app]),
        ("pkg:sysand/acme/widget", &[widget_v1, widget_v2]),
    ]);

    let solution = super::solve(
        vec![InterchangeProjectUsage::Resource {
            resource: Iri::parse("pkg:sysand/acme/app")?.into(),
            version_constraint: None,
        }],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 2);

    let install = &solution[&Identifier::from_pub_name("acme", "widget")];
    assert_eq!(install.version()?.unwrap(), "2.0.0");

    Ok(())
}

/// The same project can be used both by its resource IRI and as a directory
/// usage with the matching publisher and name; both must be satisfied by
/// the same single project in the solution
#[test]
fn directory_and_resource_usage_same_project() -> Result<(), Box<dyn std::error::Error>> {
    let app = memory_project(
        "app",
        "1.0.0",
        vec![InterchangeProjectUsageRaw::Resource {
            resource: "pkg:sysand/acme/widget".to_owned(),
            version_constraint: None,
        }],
    );
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);

    let resolver = simple_resolver_environment(&[
        ("pkg:sysand/acme/app", &[app]),
        ("pkg:sysand/acme/widget", &[widget]),
    ]);

    let solution = super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("pkg:sysand/acme/app")?.into(),
                version_constraint: None,
            },
            // Same project as `app` uses via its resource IRI
            InterchangeProjectUsage::Directory {
                dir: "some/dir".into(),
                publisher: "acme".to_owned(),
                name: "widget".to_owned(),
            },
        ],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 2);

    let install = &solution[&Identifier::from_pub_name("acme", "widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

#[test]
fn diamond_selection() -> Result<(), Box<dyn std::error::Error>> {
    let project_a_v1 = trivial_memory_project(
        "diamond_selection_a",
        "1.0.1",
        vec![("urn:kpar:diamond_selection_c", Some(">=2.0.0"))],
    );
    let project_b_v1 = trivial_memory_project(
        "diamond_selection_b",
        "1.0.2",
        vec![("urn:kpar:diamond_selection_c", Some("<3.0.0"))],
    );

    let project_c_v1 = trivial_memory_project("diamond_selection_c", "1.0.3", vec![]);
    let project_c_v2 = trivial_memory_project("diamond_selection_c", "2.0.3", vec![]);
    let project_c_v3 = trivial_memory_project("diamond_selection_c", "3.0.3", vec![]);

    let resolver = simple_resolver_environment(&[
        ("urn:kpar:diamond_selection_a", &[project_a_v1]),
        ("urn:kpar:diamond_selection_b", &[project_b_v1]),
        (
            "urn:kpar:diamond_selection_c",
            &[project_c_v1, project_c_v2, project_c_v3],
        ),
    ]);

    let solution = super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:diamond_selection_a")?.into(),
                version_constraint: Some(semver::VersionReq::parse(">=0.1.0")?),
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:diamond_selection_b")?.into(),
                version_constraint: None,
            },
        ],
        None,
        resolver,
    )?;

    assert_eq!(solution.len(), 3);

    let install_a = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:diamond_selection_a")];
    assert_eq!(install_a.version()?.unwrap(), "1.0.1");

    let install_b = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:diamond_selection_b")];
    assert_eq!(install_b.version()?.unwrap(), "1.0.2");

    let install_c = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:diamond_selection_c")];
    assert_eq!(install_c.version()?.unwrap(), "2.0.3");

    Ok(())
}

/// Version resolution must fail if two incompatible versions of the same project are requested
#[test]
fn incompatible_versions_fail() {
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);

    let resolver = simple_resolver_environment(&[("urn:kpar:widget", &[widget_v1, widget_v2])]);

    super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:widget").unwrap().into(),
                version_constraint: Some(semver::VersionReq::parse("=1.0.0").unwrap()),
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:widget").unwrap().into(),
                version_constraint: Some(semver::VersionReq::parse("=2.0.0").unwrap()),
            },
        ],
        None,
        resolver,
    )
    .unwrap_err();
}

/// Same as previous test, but for indirect dependencies
#[test]
fn incompatible_versions_fail_transitive() {
    let app_a = trivial_memory_project("app_a", "1.0.0", vec![("urn:kpar:widget", Some("=1.0.0"))]);
    let app_b = trivial_memory_project("app_b", "1.0.0", vec![("urn:kpar:widget", Some("=2.0.0"))]);
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);

    let resolver = simple_resolver_environment(&[
        ("urn:kpar:app_a", &[app_a]),
        ("urn:kpar:app_b", &[app_b]),
        ("urn:kpar:widget", &[widget_v1, widget_v2]),
    ]);

    super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:app_a").unwrap().into(),
                version_constraint: None,
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:app_b").unwrap().into(),
                version_constraint: None,
            },
        ],
        None,
        resolver,
    )
    .unwrap_err();
}

/// When the same version of a project is available from several
/// storages, the resolved dependency tree contains it exactly once
#[test]
fn single_version_single_project() -> Result<(), Box<dyn std::error::Error>> {
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);

    let storage_a = memory_resolver(&[("urn:kpar:widget", slice::from_ref(&widget))]);
    let storage_b = memory_resolver(&[("urn:kpar:widget", &[widget])]);
    let resolver = SequentialResolver::new([storage_a, storage_b]);

    let solution = super::solve(
        vec![InterchangeProjectUsage::Resource {
            resource: Iri::parse("urn:kpar:widget")?.into(),
            version_constraint: None,
        }],
        None,
        resolver,
    )?;

    assert_eq!(
        solution_projects(&solution),
        vec![("widget".to_owned(), "1.0.0".to_owned())]
    );

    Ok(())
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
struct StubError(String);

/// A project whose `version()`/`usage()` results are fully controlled, to
/// exercise the error paths of `resolve_candidates` for typed usages
#[derive(Clone, Debug)]
struct StubProject {
    version: Result<Option<String>, String>,
    usage: Result<Option<Vec<InterchangeProjectUsageRaw>>, String>,
}

impl ProjectRead for StubProject {
    type Error = StubError;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        unimplemented!("not used by the solver")
    }

    fn sources(
        &self,
        _ctx: &crate::context::ProjectContext,
    ) -> Result<Vec<crate::lock::Source>, Self::Error> {
        unimplemented!("not used by the solver")
    }

    type SourceReader<'a> = std::io::Empty;

    fn read_source<P: AsRef<crate::project::Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        unimplemented!("not used by the solver")
    }

    fn checksum_canonical_variant(&self) -> Result<crate::project::ProjectChecksum, Self::Error> {
        unimplemented!("not used by the solver")
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        self.version.clone().map_err(StubError)
    }

    fn usage(&self) -> Result<Option<Vec<InterchangeProjectUsageRaw>>, Self::Error> {
        self.usage.clone().map_err(StubError)
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
}

/// Multiple versions of the same dependency are not allowed.
/// Pubgrub does not allow multiple versions of the same project in
/// the dependency graph, unless we implement it ourselves (e.g.
/// make package identifier include a major version, so different
/// major versions will be treated as different packages by pubgrub).
/// SysMLv2 spec seemingly disallows having multiple versions of the
/// same project in the dependency graph, but we may want to support
/// it anyway
#[test]
fn usages_multiple_versions_of_same_project() {
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);

    let resolver = simple_resolver_environment(&[("urn:kpar:widget", &[widget_v1, widget_v2])]);

    super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:widget").unwrap().into(),
                version_constraint: Some(semver::VersionReq::parse("=1.0.0").unwrap()),
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:widget").unwrap().into(),
                version_constraint: Some(semver::VersionReq::parse("=2.0.0").unwrap()),
            },
        ],
        None,
        resolver,
    )
    .unwrap_err();
}

/// Transitive dependencies must not contain two different versions of the same project
#[test]
fn transitive_usages_different_versions_of_same_project() {
    let app_a = trivial_memory_project("app_a", "1.0.0", [("urn:kpar:widget", Some("=1.0.0"))]);
    let app_b = trivial_memory_project("app_b", "1.0.0", [("urn:kpar:widget", Some("=2.0.0"))]);
    let widget_v1 = trivial_memory_project("widget", "1.0.0", []);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", []);

    let resolver = simple_resolver_environment(&[
        ("urn:kpar:app_a", &[app_a]),
        ("urn:kpar:app_b", &[app_b]),
        ("urn:kpar:widget", &[widget_v1, widget_v2]),
    ]);

    super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:app_a").unwrap().into(),
                version_constraint: None,
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("urn:kpar:app_b").unwrap().into(),
                version_constraint: None,
            },
        ],
        None,
        resolver,
    )
    .unwrap_err();
}

/// A resolver whose only candidate for any usage is an error
#[derive(Debug)]
struct ErrorCandidateResolver;

impl ResolveRead for ErrorCandidateResolver {
    type Error = StubError;
    type ProjectStorage = StubProject;
    type ResolvedStorages = Vec<Result<StubProject, StubError>>;

    fn resolve_read(
        &self,
        _resolve: &ResolutionInfo,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(ResolutionOutcome::Resolved(vec![Err(StubError(
            "candidate exploded".to_owned(),
        ))]))
    }
}

/// Solve a single `acme/widget` directory usage with the given resolver and
/// return the Debug rendering of the resolution error it must produce
fn directory_usage_solve_err<R>(resolver: R) -> String
where
    R: ResolveRead + Debug + 'static,
    R::ProjectStorage: Debug,
{
    let result = super::solve(
        vec![InterchangeProjectUsage::Directory {
            dir: "some/dir".into(),
            publisher: "acme".to_owned(),
            name: "widget".to_owned(),
        }],
        None,
        resolver,
    );
    format!(
        "{:?}",
        result.expect_err("typed usage must fail resolution")
    )
}

fn stub_resolver(stub: StubProject) -> MemoryResolver<AcceptAll, StubProject> {
    MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: [(Identifier::from_pub_name("acme", "widget"), vec![stub])].into(),
    }
}

/// Typed usage resolving to an error candidate fails resolution
/// (an untyped usage would skip the candidate)
#[test]
fn typed_usage_error_candidate_fails_resolution() {
    let msg = directory_usage_solve_err(ErrorCandidateResolver);
    assert!(msg.contains("ResolvedError"), "got: {msg}");
    assert!(msg.contains("candidate exploded"), "got: {msg}");
}

/// Typed usage resolving to a project whose version is not valid semver
/// fails resolution
#[test]
fn typed_usage_invalid_version_fails_resolution() {
    let widget = trivial_memory_project("widget", "not-a-semver", vec![]);
    let resolver = memory_resolver(&[("pkg:sysand/acme/widget", &[widget])]);

    let msg = directory_usage_solve_err(resolver);
    assert!(msg.contains("InvalidResolvedVersion"), "got: {msg}");
}

/// Typed usage resolving to a project that does not expose a version
/// fails resolution
#[test]
fn typed_usage_missing_version_fails_resolution() {
    let msg = directory_usage_solve_err(stub_resolver(StubProject {
        version: Ok(None),
        usage: Ok(Some(vec![])),
    }));
    assert!(msg.contains("MissingVersion"), "got: {msg}");
}

/// Typed usage resolving to a project whose version cannot be read
/// fails resolution
#[test]
fn typed_usage_version_read_error_fails_resolution() {
    let msg = directory_usage_solve_err(stub_resolver(StubProject {
        version: Err("cannot read version".to_owned()),
        usage: Ok(Some(vec![])),
    }));
    assert!(msg.contains("VersionObtain"), "got: {msg}");
    assert!(msg.contains("cannot read version"), "got: {msg}");
}

/// Typed usage resolving to a project whose own usage list is invalid
/// fails resolution
#[test]
fn typed_usage_invalid_usages_fail_resolution() {
    let widget = memory_project(
        "widget",
        "1.0.0",
        vec![InterchangeProjectUsageRaw::Resource {
            resource: "not a valid iri".to_owned(),
            version_constraint: None,
        }],
    );
    let resolver = memory_resolver(&[("pkg:sysand/acme/widget", &[widget])]);

    let msg = directory_usage_solve_err(resolver);
    assert!(msg.contains("InvalidProject"), "got: {msg}");
}

/// Typed usage resolving to a project that does not expose its usages
/// fails resolution
#[test]
fn typed_usage_missing_usages_fails_resolution() {
    let msg = directory_usage_solve_err(stub_resolver(StubProject {
        version: Ok(Some("1.0.0".to_owned())),
        usage: Ok(None),
    }));
    assert!(msg.contains("MissingUsage"), "got: {msg}");
}

/// Typed usage resolving to a project whose usages cannot be read
/// fails resolution
#[test]
fn typed_usage_usages_read_error_fails_resolution() {
    let msg = directory_usage_solve_err(stub_resolver(StubProject {
        version: Ok(Some("1.0.0".to_owned())),
        usage: Err("cannot read usages".to_owned()),
    }));
    assert!(msg.contains("UsageObtain"), "got: {msg}");
    assert!(msg.contains("cannot read usages"), "got: {msg}");
}

/// When the project at a directory usage's path is rejected (e.g. its
/// declared publisher does not match the usage), the rejection reason must
/// surface in the solver error. A typed usage has exactly one place its
/// project can come from, so silently skipping the candidate and reporting
/// only "no valid candidates" hides the actual problem
#[cfg(feature = "filesystem")]
#[test]
fn directory_usage_candidate_rejection_reason_is_reported() -> Result<(), Box<dyn std::error::Error>>
{
    use crate::project::local_src::LocalSrcProject;

    // A real project on disk that declares a different publisher than
    // the usage expects
    let tmp = camino_tempfile::tempdir()?;
    let mut info = memory_project("widget", "1.0.0", vec![]).info.unwrap();
    info.publisher = Some("someone-else".to_owned());
    std::fs::write(
        tmp.path().join(".project.json"),
        serde_json::to_string(&info)?,
    )?;

    let project = LocalSrcProject::new_for_solve(
        tmp.path().to_owned(),
        None,
        Some("acme".to_owned()),
        "widget".to_owned(),
    );

    let resolver = MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: [(Identifier::from_pub_name("acme", "widget"), vec![project])].into(),
    };

    let result = super::solve(
        vec![InterchangeProjectUsage::Directory {
            dir: "widget".into(),
            publisher: "acme".to_owned(),
            name: "widget".to_owned(),
        }],
        None,
        resolver,
    );

    let err = result.expect_err("the only candidate declares the wrong publisher");
    // The rejection reason (`someone-else` does not match `acme`) must be part
    // of the reported error, not only visible in debug logs
    let msg = format!("{err:?}");
    assert!(
        msg.contains("someone-else"),
        "solver error should carry the candidate's rejection reason, got: {msg}"
    );

    Ok(())
}

/// A directory usage pins the project to the copy at that path. Version
/// constraints that other dependents place on the same project are checked
/// against that pinned copy, and the pinned copy is the one installed,
/// even when another source offers a higher version that would also satisfy
/// the constraints
#[test]
fn directory_usage_copy_satisfies_constraints_of_other_dependents()
-> Result<(), Box<dyn std::error::Error>> {
    let widget_local = trivial_memory_project("widget", "1.3.0", vec![]);
    let widget_index = trivial_memory_project("widget", "1.5.0", vec![]);
    let app = trivial_memory_project("app", "1.0.0", [("pkg:sysand/acme/widget", Some("^1.0"))]);

    let local_paths = memory_resolver_serving(
        Serves::DirectoryUsages,
        &[("pkg:sysand/acme/widget", &[widget_local])],
    );
    let index = memory_resolver_serving(
        Serves::ResourceUsages,
        &[
            ("pkg:sysand/acme/app", &[app]),
            ("pkg:sysand/acme/widget", &[widget_index]),
        ],
    );
    let resolver = SequentialResolver::new([local_paths, index]);

    let solution = super::solve(
        vec![
            InterchangeProjectUsage::Directory {
                dir: "some/dir".into(),
                publisher: "acme".to_owned(),
                name: "widget".to_owned(),
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("pkg:sysand/acme/app")?.into(),
                version_constraint: None,
            },
        ],
        None,
        resolver,
    )?;

    // `app`'s `^1.0` is satisfied by the pinned local copy 1.3.0; the
    // index copy 1.5.0 must not be selected over it
    let widget_versions: Vec<String> = solution_projects(&solution)
        .into_iter()
        .filter(|(name, _)| name == "widget")
        .map(|(_, version)| version)
        .collect();
    assert_eq!(widget_versions, vec!["1.3.0".to_owned()]);

    Ok(())
}

/// When the copy pinned by a directory usage cannot satisfy another
/// dependent's version constraint, resolution must fail — not silently
/// fall back to a satisfying copy of the project from another source
#[test]
fn directory_usage_copy_violating_constraints_is_an_error() -> Result<(), Box<dyn std::error::Error>>
{
    let widget_local = trivial_memory_project("widget", "1.3.0", vec![]);
    let widget_index = trivial_memory_project("widget", "2.1.0", vec![]);
    let app = trivial_memory_project(
        "app",
        "1.0.0",
        [("pkg:sysand/acme/widget", Some(">=2.0.0"))],
    );

    let local_paths = memory_resolver_serving(
        Serves::DirectoryUsages,
        &[("pkg:sysand/acme/widget", &[widget_local])],
    );
    let index = memory_resolver_serving(
        Serves::ResourceUsages,
        &[
            ("pkg:sysand/acme/app", &[app]),
            ("pkg:sysand/acme/widget", &[widget_index]),
        ],
    );
    let resolver = SequentialResolver::new([local_paths, index]);

    let result = super::solve(
        vec![
            InterchangeProjectUsage::Directory {
                dir: "some/dir".into(),
                publisher: "acme".to_owned(),
                name: "widget".to_owned(),
            },
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("pkg:sysand/acme/app")?.into(),
                version_constraint: None,
            },
        ],
        None,
        resolver,
    );

    assert!(
        result.is_err(),
        "expected resolution to fail because the directory copy (1.3.0) \
         violates `app`'s `>=2.0.0`; it must not fall back to the index copy"
    );

    Ok(())
}

/// A project reachable both by its resource IRI (available in one storage)
/// and via a directory usage (available in another storage) is still one
/// project; the resolved dependency tree contains exactly one instance of it
#[test]
fn same_project_version_from_different_storages_and_usage_forms_installs_once()
-> Result<(), Box<dyn std::error::Error>> {
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);

    let storage_a = memory_resolver(&[("pkg:sysand/acme/widget", slice::from_ref(&widget))]);
    let storage_b = memory_resolver(&[("pkg:sysand/acme/widget", &[widget])]);
    let resolver = SequentialResolver::new([storage_a, storage_b]);

    let solution = super::solve(
        vec![
            InterchangeProjectUsage::Resource {
                resource: Iri::parse("pkg:sysand/acme/widget")?.into(),
                version_constraint: None,
            },
            // The same project, used via a directory usage
            InterchangeProjectUsage::Directory {
                dir: "some/dir".into(),
                publisher: "acme".to_owned(),
                name: "widget".to_owned(),
            },
        ],
        None,
        resolver,
    )?;

    assert_eq!(
        solution_projects(&solution),
        vec![("widget".to_owned(), "1.0.0".to_owned())]
    );

    Ok(())
}
