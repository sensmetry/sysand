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
        // These fixtures stand in for an index listing several versions of a
        // project, which is what makes a default version constraint apply.
        source_may_offer_multiple_versions: true,
    }
}

/// Like [`trivial_memory_project`], but standing in for a source that names a
/// single project outright -- a path, a URL, a source override -- where there
/// is no version to choose between and so no default constraint.
fn single_project_source<'a>(
    name: &str,
    version: &str,
    usage: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> InMemoryProject {
    InMemoryProject {
        source_may_offer_multiple_versions: false,
        ..trivial_memory_project(name, version, usage)
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

#[expect(clippy::unimplemented)]
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

    fn source_may_offer_multiple_versions(&self) -> bool {
        // Like `memory_project`, this stands in for an index.
        true
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

// --- structured failures --------------------------------------------------

use std::assert_matches;

use super::SolveConflict;

fn root_usage(iri: &str, constraint: Option<&str>) -> InterchangeProjectUsage {
    InterchangeProjectUsage::Resource {
        resource: Iri::parse(iri).unwrap().into(),
        version_constraint: constraint.map(|c| semver::VersionReq::parse(c).unwrap()),
    }
}

#[test]
fn conflicts_name_root_pins_that_contradict() {
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[widget_v1, widget_v2])]);

    let err = super::solve(
        vec![
            root_usage("urn:kpar:widget", Some("=1.0.0")),
            root_usage("urn:kpar:widget", Some("=2.0.0")),
        ],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    let conflicts = err.conflicts();
    for constraint in ["=1.0.0", "=2.0.0"] {
        assert!(
            conflicts.contains(&SolveConflict::Constraint {
                iri: "urn:kpar:widget".to_owned(),
                constraint: constraint.to_owned(),
                required_by: None,
            }),
            "missing root pin `{constraint}` in {conflicts:?}"
        );
    }
}

#[test]
fn conflicts_name_the_dependents_that_pin_transitively() {
    let app_a = trivial_memory_project("app_a", "1.0.0", vec![("urn:kpar:widget", Some("=1.0.0"))]);
    let app_b = trivial_memory_project("app_b", "1.0.0", vec![("urn:kpar:widget", Some("=2.0.0"))]);
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);
    let resolver = memory_resolver(&[
        ("urn:kpar:app_a", &[app_a]),
        ("urn:kpar:app_b", &[app_b]),
        ("urn:kpar:widget", &[widget_v1, widget_v2]),
    ]);

    let err = super::solve(
        vec![
            root_usage("urn:kpar:app_a", None),
            root_usage("urn:kpar:app_b", None),
        ],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    let conflicts = err.conflicts();
    // The constraint is the *dependent's*, even though pubgrub interns the
    // `widget` package once (by identifier) with whichever usage came first.
    for (dependent, constraint) in [("urn:kpar:app_a", "=1.0.0"), ("urn:kpar:app_b", "=2.0.0")] {
        assert!(
            conflicts.contains(&SolveConflict::Constraint {
                iri: "urn:kpar:widget".to_owned(),
                constraint: constraint.to_owned(),
                required_by: Some(dependent.to_owned()),
            }),
            "missing `{dependent}` pin `{constraint}` in {conflicts:?}"
        );
    }
    assert!(
        conflicts
            .iter()
            .all(|c| !matches!(c, SolveConflict::NotFound { .. })),
        "{conflicts:?}"
    );
}

/// A constraint that nothing satisfies makes the project that stated it
/// unusable, which is a solve failure rather than a retrieval one: the
/// project was retrieved, it just cannot be used. Where the root states the
/// constraint there is nothing to backtrack to, so the solve still fails --
/// with one cause, named in full.
#[test]
fn no_matching_version_is_a_no_versions_conflict_with_the_found_versions() {
    let widget_v1 = trivial_memory_project("widget", "1.0.0", vec![]);
    let widget_v2 = trivial_memory_project("widget", "2.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[widget_v1, widget_v2])]);

    let err = super::solve(
        vec![root_usage("urn:kpar:widget", Some(">=3"))],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    assert_eq!(
        err.conflicts(),
        vec![SolveConflict::NoVersions {
            iri: "urn:kpar:widget".to_owned(),
            constraint: ">=3".to_owned(),
            defaulted: false,
            found: vec!["1.0.0".to_owned(), "2.0.0".to_owned()],
            required_by: None,
        }]
    );
    assert_eq!(
        err.to_string(),
        "requested version unavailable: project `urn:kpar:widget`\n\
         was found, but the requested version constraint `>=3`\n\
         was not satisfied by any of the found versions:\n\
         `1.0.0`, `2.0.0`"
    );
}

/// Where the constraint comes from a dependency rather than the root, the
/// report names the dependency that could not be used alongside the cause,
/// and `conflicts()` attributes it with `required_by`.
#[test]
fn no_matching_version_for_a_dependency_names_the_dependent_in_the_report() {
    let app = trivial_memory_project("app", "1.0.0", vec![("urn:kpar:widget", Some("^2"))]);
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:app", &[app]), ("urn:kpar:widget", &[widget])]);

    let err = super::solve(vec![root_usage("urn:kpar:app", None)], None, resolver).unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    assert!(
        err.conflicts().contains(&SolveConflict::NoVersions {
            iri: "urn:kpar:widget".to_owned(),
            constraint: "^2".to_owned(),
            defaulted: false,
            found: vec!["1.0.0".to_owned()],
            required_by: Some("urn:kpar:app".to_owned()),
        }),
        "got: {:?}",
        err.conflicts()
    );

    // More than one cause, so the report is pubgrub's own, and both projects
    // are in it
    let report = err.to_string();
    for expected in [
        "failed to satisfy usage constraints:",
        "IRI `urn:kpar:app`",
        "depends on IRI `urn:kpar:widget` (^2)",
    ] {
        assert!(
            report.contains(expected),
            "`{expected}` missing from:\n{report}"
        );
    }
}

#[test]
fn found_versions_are_sorted_by_semver_and_listed_once() {
    // Resolver order is neither sorted nor free of duplicates: two sources
    // may offer the same release, and `1.10.0` must sort after `1.9.0`.
    let widgets: Vec<_> = ["2.0.0", "1.10.0", "1.9.0", "2.0.0"]
        .into_iter()
        .map(|v| trivial_memory_project("widget", v, vec![]))
        .collect();
    let resolver = memory_resolver(&[("urn:kpar:widget", &widgets)]);

    let err = super::solve(
        vec![root_usage("urn:kpar:widget", Some(">=3"))],
        None,
        resolver,
    )
    .unwrap_err();

    assert_matches!(
        err.conflicts().as_slice(),
        [SolveConflict::NoVersions { found, .. }] if *found == ["1.9.0", "1.10.0", "2.0.0"]
    );
}

#[test]
fn unknown_project_is_a_not_found_conflict() {
    let resolver = memory_resolver(&[]);

    let err = super::solve(vec![root_usage("urn:kpar:absent", None)], None, resolver).unwrap_err();

    assert_eq!(err.kind(), "retrieval");
    assert!(err.resolution_error().is_none());
    assert_matches!(
        err.conflicts().as_slice(),
        [SolveConflict::NotFound { iri, .. }] if iri == "urn:kpar:absent"
    );
}

// --- prerelease versions --------------------------------------------------
//
// Constraint matching goes through `semver::VersionReq`, which implements
// cargo's rule: a prerelease is only ever selected by a constraint that
// itself names a prerelease of the same `major.minor.patch`.

/// A constraint that names no prerelease never selects one, even when the
/// prerelease is the highest version published.
#[test]
fn prerelease_is_ignored_by_a_release_constraint() -> Result<(), Box<dyn std::error::Error>> {
    let prerelease = trivial_memory_project("widget", "1.1.0-alpha.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[prerelease, release])]);

    let solution = super::solve(
        vec![root_usage("urn:kpar:widget", Some("^1"))],
        None,
        resolver,
    )?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// `*` is not an opt-in either: like cargo's `*`, it selects releases only.
#[test]
fn star_constraint_ignores_prereleases() -> Result<(), Box<dyn std::error::Error>> {
    let prerelease = trivial_memory_project("widget", "2.0.0-beta.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[prerelease, release])]);

    let solution = super::solve(
        vec![root_usage("urn:kpar:widget", Some("*"))],
        None,
        resolver,
    )?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// A constraint that names a prerelease opts in to prereleases of that
/// `major.minor.patch`.
#[test]
fn prerelease_is_selected_when_the_constraint_names_one() -> Result<(), Box<dyn std::error::Error>>
{
    let alpha = trivial_memory_project("widget", "1.1.0-alpha.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[alpha, release])]);

    let solution = super::solve(
        vec![root_usage("urn:kpar:widget", Some("^1.1.0-alpha"))],
        None,
        resolver,
    )?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.1.0-alpha.1");

    Ok(())
}

/// The opt-in is per release: a prerelease of a *different* release stays
/// excluded, even one that is numerically greater than everything allowed.
#[test]
fn prerelease_constraint_does_not_admit_other_prereleases() -> Result<(), Box<dyn std::error::Error>>
{
    let beta = trivial_memory_project("widget", "2.0.0-beta.1", vec![]);
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[beta, alpha])]);

    let solution = super::solve(
        vec![root_usage("urn:kpar:widget", Some(">=1.0.0-alpha"))],
        None,
        resolver,
    )?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0-alpha.1");

    Ok(())
}

/// When only prereleases are published, a release constraint matches nothing
/// and the prereleases are reported as the versions that were found.
#[test]
fn only_prereleases_published_is_a_no_versions_failure() {
    let beta = trivial_memory_project("widget", "1.0.0-beta.1", vec![]);
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[beta, alpha])]);

    let err = super::solve(
        vec![root_usage("urn:kpar:widget", Some("^1"))],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    assert_eq!(
        err.conflicts(),
        vec![SolveConflict::NoVersions {
            iri: "urn:kpar:widget".to_owned(),
            constraint: "^1".to_owned(),
            defaulted: false,
            found: vec!["1.0.0-alpha.1".to_owned(), "1.0.0-beta.1".to_owned()],
            required_by: None,
        }]
    );
}

/// The rule applies to a constraint coming from a dependency just as it does
/// to one written in the root project.
#[test]
fn transitive_constraint_ignores_prereleases() -> Result<(), Box<dyn std::error::Error>> {
    let app = trivial_memory_project("app", "1.0.0", vec![("urn:kpar:widget", Some("^1"))]);
    let prerelease = trivial_memory_project("widget", "1.1.0-alpha.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[
        ("urn:kpar:app", &[app]),
        ("urn:kpar:widget", &[prerelease, release]),
    ]);

    let solution = super::solve(vec![root_usage("urn:kpar:app", Some("^1"))], None, resolver)?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// A prerelease named by one dependent does not leak into the constraint of
/// another: opting in is not transitive across dependents.
#[test]
fn prerelease_opt_in_of_one_dependent_conflicts_with_a_release_pin() {
    let app_a = trivial_memory_project(
        "app_a",
        "1.0.0",
        vec![("urn:kpar:widget", Some("=1.0.0-alpha.1"))],
    );
    let app_b = trivial_memory_project("app_b", "1.0.0", vec![("urn:kpar:widget", Some("^1"))]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let resolver = memory_resolver(&[
        ("urn:kpar:app_a", &[app_a]),
        ("urn:kpar:app_b", &[app_b]),
        ("urn:kpar:widget", &[release, alpha]),
    ]);

    let err = super::solve(
        vec![
            root_usage("urn:kpar:app_a", None),
            root_usage("urn:kpar:app_b", None),
        ],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
}

// --- the default constraint -----------------------------------------------

/// An unconstrained PURL usage takes `DEFAULT_INDEX_CONSTRAINT`, so it ignores
/// prereleases exactly as a written-out `*` does.
#[test]
fn unconstrained_purl_usage_ignores_prereleases() -> Result<(), Box<dyn std::error::Error>> {
    // Index order: descending, so the highest release is the first candidate
    // the default admits.
    let candidates: Vec<_> = ["3.0.0-beta.1", "2.0.0", "1.0.0"]
        .into_iter()
        .map(|v| trivial_memory_project("widget", v, vec![]))
        .collect();
    let resolver = memory_resolver(&[("pkg:sysand/acme/widget", &candidates)]);

    let solution = super::solve(
        vec![root_usage("pkg:sysand/acme/widget", None)],
        None,
        resolver,
    )?;

    let install = &solution[&Identifier::from_iri_unchecked_str("pkg:sysand/acme/widget")];
    assert_eq!(install.version()?.unwrap(), "2.0.0");

    Ok(())
}

/// An unconstrained PURL usage of a project that has only ever had
/// prereleases published fails the way `*` does, naming `*` as the constraint
/// that went unsatisfied -- but saying that `*` is the default, since a reader
/// told a `*` went unsatisfied would go looking for one they never wrote.
#[test]
fn unconstrained_purl_usage_of_a_prerelease_only_project_is_a_no_versions_failure() {
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let resolver = memory_resolver(&[("pkg:sysand/acme/widget", &[alpha])]);

    let err = super::solve(
        vec![root_usage("pkg:sysand/acme/widget", None)],
        None,
        resolver,
    )
    .unwrap_err();

    assert_eq!(err.kind(), "no_solution");
    assert_eq!(
        err.conflicts(),
        vec![SolveConflict::NoVersions {
            iri: "pkg:sysand/acme/widget".to_owned(),
            constraint: "*".to_owned(),
            defaulted: true,
            found: vec!["1.0.0-alpha.1".to_owned()],
            required_by: None,
        }]
    );
    assert_eq!(
        err.to_string(),
        "no usable version: project `pkg:sysand/acme/widget`\n\
         was found, but the default version constraint `*`\n\
         was not satisfied by any of the found versions:\n\
         `1.0.0-alpha.1`"
    );
}

/// A PURL dependency that states no constraint takes the default just as a
/// root usage does.
#[test]
fn unconstrained_transitive_purl_usage_ignores_prereleases()
-> Result<(), Box<dyn std::error::Error>> {
    let app = trivial_memory_project("app", "1.0.0", vec![("pkg:sysand/acme/widget", None)]);
    let prerelease = trivial_memory_project("widget", "2.0.0-beta.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[
        ("urn:kpar:app", &[app]),
        ("pkg:sysand/acme/widget", &[prerelease, release]),
    ]);

    let solution = super::solve(vec![root_usage("urn:kpar:app", None)], None, resolver)?;

    let install = &solution[&Identifier::from_iri_unchecked_str("pkg:sysand/acme/widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// The default follows the source, not the IRI. A source that names one
/// project -- a local path, an HTTP URL, a git repository, a source override
/// -- has no version to choose between, so whatever is there is taken as it
/// is, prerelease and all. Defaulting to `*` would refuse such a project
/// outright, and no constraint the user could add to the usage would express
/// "whatever is at this location" again. That holds for a `pkg:sysand` IRI
/// pinned to such a source just as much as for any other form.
#[test]
fn unconstrained_usage_of_a_single_project_source_admits_a_prerelease()
-> Result<(), Box<dyn std::error::Error>> {
    for iri in [
        "pkg:sysand/acme/widget",
        "urn:kpar:widget",
        "file:///home/someone/widget",
        "https://example.com/widget",
        "git+https://example.com/widget.git",
    ] {
        let prerelease = single_project_source("widget", "1.0.0-alpha.1", vec![]);
        let resolver = memory_resolver(&[(iri, &[prerelease])]);

        let solution = super::solve(vec![root_usage(iri, None)], None, resolver)?;

        let install = &solution[&Identifier::from_iri_unchecked_str(iri)];
        assert_eq!(install.version()?.unwrap(), "1.0.0-alpha.1", "for `{iri}`");
    }

    Ok(())
}

/// And the other way round: an opaque `urn:kpar:` IRI that an index advertises
/// takes the default like any other index usage. The IRI form says nothing
/// about whether there was a choice to make.
#[test]
fn unconstrained_usage_of_a_multi_version_source_ignores_prereleases()
-> Result<(), Box<dyn std::error::Error>> {
    let release = trivial_memory_project("widget", "1.0.0", vec![]);
    let prerelease = trivial_memory_project("widget", "2.0.0-beta.1", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[prerelease, release])]);

    let solution = super::solve(vec![root_usage("urn:kpar:widget", None)], None, resolver)?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// With no constraint and no default, `selected_by` selects every candidate,
/// which is the cofinite case `choose_version()` answers with the highest
/// version -- prerelease included.
#[test]
fn unconstrained_usage_of_a_single_project_source_takes_the_highest_version()
-> Result<(), Box<dyn std::error::Error>> {
    let release = single_project_source("widget", "1.0.0", vec![]);
    let prerelease = single_project_source("widget", "2.0.0-beta.1", vec![]);
    let resolver = memory_resolver(&[("urn:kpar:widget", &[release, prerelease])]);

    let solution = super::solve(vec![root_usage("urn:kpar:widget", None)], None, resolver)?;

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(install.version()?.unwrap(), "2.0.0-beta.1");

    Ok(())
}

/// Directory and `.kpar` path usages carry no constraint at all and never
/// reach `selected_by`: like a cargo path dependency, they take whatever
/// version the project at that location has, prerelease or not.
#[test]
fn directory_usage_admits_a_prerelease() -> Result<(), Box<dyn std::error::Error>> {
    let widget = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
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

    let install = &solution[&Identifier::from_pub_name("acme", "widget")];
    assert_eq!(install.version()?.unwrap(), "1.0.0-alpha.1");

    Ok(())
}

// --- candidate order ------------------------------------------------------

/// Once any constraint applies — stated or defaulted — selection runs through
/// the `DiscreteHashSet::Finite` arm of `choose_version()`, which takes
/// `min()` over candidate indices: the first matching candidate the resolver
/// listed, not the highest matching version.
///
/// Candidate order is a preference rank, not an accident — `CombinedResolver`
/// emits authoritative sources first and appends unmatched local cache copies
/// last — so ranking by index is the right thing to do. What it relies on is
/// every source listing its own versions in descending order, which the index
/// protocol enforces (see
/// `index_order_makes_the_lowest_candidate_index_the_highest_version`) but
/// `LocalDirectoryEnvironment` does not: it yields install order.
///
/// Spelled out here with prereleases because that is where it bites hardest:
/// out of descending order, `^1.0.0-alpha` picks `1.0.0-alpha.1` over the
/// finished `1.0.0`.
#[test]
fn constrained_usage_picks_by_candidate_order_not_by_version()
-> Result<(), Box<dyn std::error::Error>> {
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let release = trivial_memory_project("widget", "1.0.0", vec![]);

    for (candidates, expected) in [
        ([alpha.clone(), release.clone()], "1.0.0-alpha.1"),
        ([release, alpha], "1.0.0"),
    ] {
        let resolver = memory_resolver(&[("urn:kpar:widget", &candidates)]);

        let solution = super::solve(
            vec![root_usage("urn:kpar:widget", Some("^1.0.0-alpha"))],
            None,
            resolver,
        )?;

        let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
        assert_eq!(install.version()?.unwrap(), expected);
    }

    Ok(())
}

/// Documents a known shortcoming: within one source, the highest matching
/// version should win, and today it does not.
///
/// Candidate order *across* sources is deliberate — `CombinedResolver` ranks
/// authoritative sources ahead of local cache copies, and `choose_version()`
/// honouring that rank is correct. What is missing is the other half: each
/// source should list its own versions in descending order, so that the rank
/// and the version agree. The index protocol requires exactly that, but
/// `LocalDirectoryEnvironment::versions` yields `sysand_env.json` install
/// order, which this fixture stands in for — install `1.0.0`, then `1.2.0`,
/// ask for `^1`, and resolution settles on `1.0.0`.
///
/// It decides anything only where the local environment is the sole source of
/// a project: offline, or a project no index advertises. Anywhere an
/// authoritative source also has it, matching versions are folded together by
/// checksum and the leftovers are last-resort by design.
///
/// The fix belongs in the environment, not here — sorting in
/// `choose_version()` would flatten the cross-source rank and let a stale
/// cached copy outrank an index. Drop the `#[should_panic]` when the
/// environment sorts.
#[test]
#[should_panic(expected = "resolved to `1.0.0`, not the highest matching version")]
fn constrained_usage_should_pick_the_highest_matching_version_within_one_source() {
    // The order a local environment hands over: install order, not descending.
    let candidates = ["1.0.0", "1.2.0"].map(|v| trivial_memory_project("widget", v, vec![]));
    let resolver = memory_resolver(&[("urn:kpar:widget", &candidates)]);

    let solution = super::solve(
        vec![root_usage("urn:kpar:widget", Some("^1"))],
        None,
        resolver,
    )
    .unwrap();

    let install = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    let version = install.version().unwrap().unwrap();
    assert_eq!(
        version, "1.2.0",
        "resolved to `{version}`, not the highest matching version"
    );
}

/// Why the preceding shortcoming is invisible over an index: `versions.json`
/// MUST be in strictly descending semver precedence (`validate_versions`
/// rejects anything else with `VersionsOutOfOrder`), and `EnvResolver` hands
/// that order to the solver unchanged, so candidate index 0 is the highest
/// version and `choose_version()`'s lowest-index pick lands on it.
///
/// The two rules are load-bearing together: relaxing the ordering rule at the
/// protocol boundary would silently change which version gets resolved.
/// Descending order also puts a release ahead of its own prereleases, since
/// `1.0.0-alpha.1 < 1.0.0`, so an opt-in constraint still prefers the release.
#[test]
fn index_order_makes_the_lowest_candidate_index_the_highest_version()
-> Result<(), Box<dyn std::error::Error>> {
    // Candidates in the order an index advertises them.
    let versions = ["2.0.0", "1.1.0", "1.0.0", "1.0.0-alpha.1"];
    let parsed: Vec<semver::Version> = versions
        .iter()
        .map(|v| semver::Version::parse(v).unwrap())
        .collect();
    assert!(
        parsed.is_sorted_by(|v1, v2| v1 > v2),
        "fixture must be in the strictly descending order the index protocol requires"
    );
    let candidates: Vec<_> = versions
        .into_iter()
        .map(|v| trivial_memory_project("widget", v, vec![]))
        .collect();

    for (constraint, expected) in [
        (Some("^1"), "1.1.0"),
        (Some("^1.0.0-alpha"), "1.1.0"),
        (Some("*"), "2.0.0"),
        // The default `*` that an unconstrained PURL usage takes.
        (None, "2.0.0"),
    ] {
        let resolver = memory_resolver(&[("pkg:sysand/acme/widget", &candidates)]);

        let solution = super::solve(
            vec![root_usage("pkg:sysand/acme/widget", constraint)],
            None,
            resolver,
        )?;

        let install = &solution[&Identifier::from_iri_unchecked_str("pkg:sysand/acme/widget")];
        assert_eq!(
            install.version()?.unwrap(),
            expected,
            "for `{constraint:?}`"
        );
    }

    Ok(())
}

// --- backtracking ---------------------------------------------------------

/// A constraint that selects none of the candidates is handed to pubgrub as
/// an empty dependency set, which rules out only the candidate that stated it.
/// An `Err` out of `get_dependencies` would instead abort `pubgrub::resolve`
/// outright, so the solver would never try the other candidates. It does:
/// it backtracks past a dependency whose own constraint cannot be met rather
/// than failing a solve that has an answer.
///
/// Here `app` 2.0.0 wants a `widget` that was never published, while `app`
/// 1.0.0 wants one that was: `app` 1.0.0 with `widget` 1.0.0 is the solution.
#[test]
fn solve_backtracks_past_a_constraint_that_selects_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    // Index order: descending, so `app` 2.0.0 is the candidate tried first.
    let app_v2 = trivial_memory_project("app", "2.0.0", vec![("urn:kpar:widget", Some("^2"))]);
    let app_v1 = trivial_memory_project("app", "1.0.0", vec![("urn:kpar:widget", Some("^1"))]);
    // `widget` 2.x was never published.
    let widget = trivial_memory_project("widget", "1.0.0", vec![]);
    let resolver = memory_resolver(&[
        ("urn:kpar:app", &[app_v2, app_v1]),
        ("urn:kpar:widget", &[widget]),
    ]);

    let solution = super::solve(vec![root_usage("urn:kpar:app", Some("*"))], None, resolver)?;

    let app = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:app")];
    assert_eq!(app.version()?.unwrap(), "1.0.0");
    let widget = &solution[&Identifier::from_iri_unchecked_str("urn:kpar:widget")];
    assert_eq!(widget.version()?.unwrap(), "1.0.0");

    Ok(())
}

/// The same, reached through the default constraint rather than a written
/// one: `app` 2.0.0 depends on a `widget` that exists but has only ever been
/// published as a prerelease, so the default `*` selects nothing. `app` 1.0.0
/// needs no `widget` at all.
#[test]
fn solve_backtracks_past_a_default_constraint_that_selects_nothing()
-> Result<(), Box<dyn std::error::Error>> {
    let app_v2 = trivial_memory_project("app", "2.0.0", vec![("pkg:sysand/acme/widget", None)]);
    let app_v1 = trivial_memory_project("app", "1.0.0", vec![]);
    let alpha = trivial_memory_project("widget", "1.0.0-alpha.1", vec![]);
    let resolver = memory_resolver(&[
        ("pkg:sysand/acme/app", &[app_v2, app_v1]),
        ("pkg:sysand/acme/widget", &[alpha]),
    ]);

    let solution = super::solve(
        vec![root_usage("pkg:sysand/acme/app", None)],
        None,
        resolver,
    )?;

    let app = &solution[&Identifier::from_iri_unchecked_str("pkg:sysand/acme/app")];
    assert_eq!(app.version()?.unwrap(), "1.0.0");
    assert_eq!(solution.len(), 1);

    Ok(())
}

// Backtracking does not paper over a genuinely unsolvable case: with no
// alternative candidate to fall back to, the solve still fails and names the
// constraint that could not be met -- see
// `no_matching_version_for_a_dependency_names_the_dependent_in_the_report`.
