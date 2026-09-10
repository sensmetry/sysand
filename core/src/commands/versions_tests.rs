// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{assert_matches, collections::HashMap};

use fluent_uri::Iri;
use indexmap::IndexMap;
use semver::Version;

use super::{VersionListing, do_versions};
use crate::{
    info::InfoError,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{memory::InMemoryProject, utils::Identifier},
    resolve::memory::{AcceptAll, MemoryResolver},
};

const IRI: &str = "urn:kpar:listed";

fn project(version: &str) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: "listed".to_owned(),
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
    }
}

fn resolver(candidates: Vec<InMemoryProject>) -> MemoryResolver<AcceptAll, InMemoryProject> {
    MemoryResolver::from_iter([(Identifier::from_iri_unchecked_str(IRI), candidates)])
}

fn iri() -> Iri<String> {
    Iri::parse(IRI).unwrap().into()
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

#[test]
fn lists_distinct_semver_versions_highest_first_and_keeps_the_rest() {
    let resolver = resolver(vec![
        project("0.10.3"),
        project("nightly"),
        project("1.2.3"),
        project("0.10.3"),
    ]);

    let listing = do_versions(&iri(), &resolver).unwrap();

    assert_eq!(
        listing,
        VersionListing {
            iri: IRI.to_owned(),
            versions: vec![v("1.2.3"), v("0.10.3")],
            ignored: vec!["nightly".to_owned()],
        }
    );
}

#[test]
fn only_non_semver_versions_is_not_an_error() {
    let resolver = resolver(vec![project("nightly"), project("latest")]);

    let listing = do_versions(&iri(), &resolver).unwrap();

    assert!(listing.versions.is_empty());
    assert_eq!(listing.ignored, vec!["nightly", "latest"]);
}

#[test]
fn candidates_without_a_version_are_skipped() {
    let resolver = resolver(vec![InMemoryProject::default(), project("1.0.0")]);

    let listing = do_versions(&iri(), &resolver).unwrap();

    assert_eq!(listing.versions, vec![v("1.0.0")]);
    assert!(listing.ignored.is_empty());
}

#[test]
fn nothing_usable_is_no_semantic_versions_found() {
    let resolver = resolver(vec![InMemoryProject::default()]);

    assert_matches!(
        do_versions(&iri(), &resolver).unwrap_err(),
        InfoError::NoSemanticVersionsFound(found) if found.is_empty()
    );
}

#[test]
fn unknown_iri_is_not_found() {
    let resolver = resolver(vec![project("1.0.0")]);
    let other: Iri<String> = Iri::parse("urn:kpar:other").unwrap().into();

    assert_matches!(
        do_versions(&other, &resolver).unwrap_err(),
        InfoError::NotFound { .. }
    );
}
