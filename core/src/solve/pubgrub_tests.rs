// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, slice};

use fluent_uri::Iri;
use indexmap::IndexMap;

use crate::{
    env::memory::MemoryStorageEnvironment,
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage,
        InterchangeProjectUsageRaw,
    },
    project::{ProjectRead, memory::InMemoryProject, utils::Identifier},
    resolve::{
        env::EnvResolver,
        memory::{AcceptAll, MemoryResolver},
        sequential::SequentialResolver,
    },
};

fn trivial_memory_project(
    name: &str,
    version: &str,
    usage: Vec<(&str, Option<&str>)>,
) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: name.to_string(),
            publisher: None,
            description: None,
            version: version.to_string(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: usage
                .into_iter()
                .map(|(d, dv)| InterchangeProjectUsageRaw::Resource {
                    resource: d.to_string(),
                    version_constraint: dv.map(|x| x.to_string()),
                })
                .collect(),
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::default(),
            created: "123".to_string(),
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
            .map(|(id, projs)| {
                (
                    Identifier::from(Iri::parse(id.to_string()).unwrap()),
                    projs.to_vec(),
                )
            })
            .collect(),
    }
}

/// The `(name, version)` pairs of all projects in a solution
fn solution_projects<P: ProjectRead>(solution: &HashMap<Iri<String>, P>) -> Vec<(String, String)> {
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

    let solution = super::solve(vec![], resolver)?;

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
            resource: fluent_uri::Iri::parse("urn:kpar:version_selection")?.into(),
            version_constraint: Some(semver::VersionReq::parse(">=2.0.0")?),
        }],
        resolver,
    )?;

    assert_eq!(solution.len(), 1);

    let install = solution
        .get(Iri::parse("urn:kpar:version_selection")?.into())
        .unwrap();

    assert_eq!(install.version()?.unwrap(), "2.0.1");

    Ok(())
}

#[test]
fn version_constraint_default() -> Result<(), Box<dyn std::error::Error>> {
    // `semver` by default prepends `^` if a version requirement does not
    // have a comparator. This is not documented, but is also extremely
    // unlikely to change, as it's the behavior relied on by cargo
    let v_no_caret = semver::VersionReq::parse("2.0.0")?;
    let v_caret = semver::VersionReq::parse("^2.0.0")?;
    assert_eq!(v_no_caret, v_caret);

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
                resource: fluent_uri::Iri::parse("urn:kpar:diamond_selection_a")?.into(),
                version_constraint: Some(semver::VersionReq::parse(">=0.1.0")?),
            },
            InterchangeProjectUsage::Resource {
                resource: fluent_uri::Iri::parse("urn:kpar:diamond_selection_b")?.into(),
                version_constraint: None,
            },
        ],
        resolver,
    )?;

    assert_eq!(solution.len(), 3);

    let install_a = solution
        .get(Iri::parse("urn:kpar:diamond_selection_a")?.into())
        .unwrap();
    assert_eq!(install_a.version()?.unwrap(), "1.0.1");

    let install_b = solution
        .get(Iri::parse("urn:kpar:diamond_selection_b")?.into())
        .unwrap();
    assert_eq!(install_b.version()?.unwrap(), "1.0.2");

    let install_c = solution
        .get(Iri::parse("urn:kpar:diamond_selection_c")?.into())
        .unwrap();
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
        resolver,
    )?;

    assert_eq!(
        solution_projects(&solution),
        vec![("widget".to_string(), "1.0.0".to_string())]
    );

    Ok(())
}
