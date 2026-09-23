// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Which version [`get_project_version`] picks. It is used in
//! `sysand clone <iri>` and `sysand env install --no-deps`.

use std::collections::HashMap;

use indexmap::IndexMap;
use semver::VersionReq;
use sysand_core::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{memory::InMemoryProject, utils::Identifier},
    resolve::{
        ResolutionInfo,
        memory::{AcceptAll, MemoryResolver},
    },
};

use super::get_project_version;

const IRI: &str = "urn:kpar:widget";
const PURL: &str = "pkg:sysand/acme/widget";

fn project(version: &str) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "widget".to_owned(),
            publisher: None,
            description: None,
            version: version.to_owned(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::new(),
            created: "2026-01-01T00:00:00Z".to_owned(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }),
        files: HashMap::new(),
        nominal_sources: vec![],
        // These fixtures stand in for an index listing several versions of a
        // project, which is what makes a default version constraint apply.
        source_may_offer_multiple_versions: true,
    }
}

/// The same project as a source that names it outright would offer it -- a
/// path or a source override, where there is no version to choose between.
fn single_project_source(version: &str) -> InMemoryProject {
    InMemoryProject {
        source_may_offer_multiple_versions: false,
        ..project(version)
    }
}

/// Candidates for `iri`, in the order a resolver would offer them.
fn resolver(iri: &str, versions: &[&str]) -> MemoryResolver<AcceptAll, InMemoryProject> {
    MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(iri),
        versions.iter().map(|v| project(v)).collect::<Vec<_>>(),
    )])
}

/// The version `get_project_version` picks out of `versions` for `iri`, where
/// the source names one project outright and so carries no default.
fn picked_from_single_project_source(iri: &str, versions: &[&str]) -> Result<String, String> {
    let resolve = ResolutionInfo::iri(fluent_uri::Iri::parse(iri.to_owned()).unwrap());
    let resolver = MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(iri),
        versions.iter().map(|v| single_project_source(v)).collect(),
    )]);
    get_project_version(&resolve, None, &resolver)
        .map(|(version, _storage)| version.to_string())
        .map_err(|err| err.to_string())
}

/// The version `get_project_version` picks out of `versions` for `iri`.
fn picked(iri: &str, version: Option<&str>, versions: &[&str]) -> Result<String, String> {
    let resolve = ResolutionInfo::iri(fluent_uri::Iri::parse(iri.to_owned()).unwrap());
    get_project_version(
        &resolve,
        version.map(|v| VersionReq::parse(v).unwrap()),
        &resolver(iri, versions),
    )
    .map(|(version, _storage)| version.to_string())
    .map_err(|err| err.to_string())
}

#[test]
fn highest_version_is_picked() {
    assert_eq!(
        picked(IRI, None, &["1.0.0", "2.0.0", "0.10.3"]).unwrap(),
        "2.0.0"
    );
}

#[test]
fn a_named_version_is_picked_over_higher_ones() {
    assert_eq!(
        picked(IRI, Some("1.0.0"), &["2.0.0", "1.0.0"]).unwrap(),
        "1.0.0"
    );
}

/// An unconstrained PURL takes the default `*`, so the prerelease loses to
/// the highest release even though it sorts above it.
#[test]
fn purl_without_a_version_ignores_prereleases() {
    assert_eq!(
        picked(PURL, None, &["3.0.0-beta.1", "2.0.0", "1.0.0"]).unwrap(),
        "2.0.0"
    );
}

/// The default follows the source, not the IRI: a `pkg:sysand` IRI pinned to a
/// source that names one project takes what is there, prerelease and all. The
/// solver agrees
/// (`unconstrained_usage_of_a_single_project_source_admits_a_prerelease`).
#[test]
fn purl_pinned_to_a_single_project_source_admits_a_prerelease() {
    let resolve = ResolutionInfo::iri(fluent_uri::Iri::parse(PURL.to_owned()).unwrap());
    let resolver = MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(PURL),
        vec![single_project_source("1.0.0-alpha.1")],
    )]);

    let (version, _storage) = get_project_version(&resolve, None, &resolver).unwrap();

    assert_eq!(version.to_string(), "1.0.0-alpha.1");
}

/// Naming the prerelease is the opt-in, as `--version` is for `cargo
/// install`.
#[test]
fn purl_with_a_named_prerelease_picks_it() {
    assert_eq!(
        picked(PURL, Some("3.0.0-beta.1"), &["3.0.0-beta.1", "2.0.0"]).unwrap(),
        "3.0.0-beta.1"
    );
}

/// A PURL project that has only ever published prereleases fails rather than
/// installing one behind the user's back — the solver fails here too
/// (`unconstrained_purl_usage_of_a_prerelease_only_project_is_a_no_versions_failure`).
/// The versions found are named, highest first, so the user can pass one.
#[test]
fn purl_with_only_prereleases_is_an_error_naming_them() {
    let err = picked(PURL, None, &["1.0.0-alpha.1", "1.0.0-beta.1"]).unwrap_err();

    assert!(
        err.contains("only pre-releases (1.0.0-beta.1, 1.0.0-alpha.1)"),
        "got: {err}"
    );
    assert!(
        err.contains("pass the version to use one of them"),
        "got: {err}"
    );
}

/// A source that names one project carries no default, so the highest version
/// wins whether or not it is a prerelease — as it does in the solver
/// (`unconstrained_usage_of_a_single_project_source_takes_the_highest_version`).
#[test]
fn single_project_source_takes_the_highest_version_including_a_prerelease() {
    assert_eq!(
        picked_from_single_project_source(IRI, &["2.0.0-beta.1", "1.0.0"]).unwrap(),
        "2.0.0-beta.1"
    );
}

/// Which is what makes cloning a prerelease-only project by location work:
/// no constraint to fail against.
#[test]
fn prerelease_only_project_is_picked_from_a_single_project_source() {
    assert_eq!(
        picked_from_single_project_source(IRI, &["1.0.0-alpha.1"]).unwrap(),
        "1.0.0-alpha.1"
    );
}

/// An opaque `urn:kpar:` IRI an index advertises takes the default like any
/// other index usage — the IRI form says nothing about whether there was a
/// choice to make.
#[test]
fn non_purl_iri_from_a_multi_version_source_ignores_prereleases() {
    assert_eq!(
        picked(IRI, None, &["2.0.0-beta.1", "1.0.0"]).unwrap(),
        "1.0.0"
    );
}
