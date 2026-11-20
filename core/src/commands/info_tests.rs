// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{assert_matches, collections::HashMap};

use fluent_uri::Iri;
use indexmap::IndexMap;

use super::{InfoError, do_info};
use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{memory::InMemoryProject, utils::Identifier},
    resolve::memory::{AcceptAll, MemoryResolver},
};

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
            index: Some(IndexMap::new()),
            root: None,
            exports: None,
            created: "2026-01-01T00:00:00Z".to_owned(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }),
        model_toml: None,
        files: HashMap::new(),
        nominal_sources: vec![],
        source_may_offer_multiple_versions: false,
    }
}

/// Candidates for `iri`, in the order a resolver would offer them.
fn resolver(iri: &str, versions: &[&str]) -> MemoryResolver<AcceptAll, InMemoryProject> {
    MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(iri),
        versions.iter().map(|v| project(v)).collect::<Vec<_>>(),
    )])
}

fn iri(iri: &str) -> Iri<String> {
    Iri::parse(iri.to_owned()).unwrap()
}

/// The baseline: out of several candidates `info` describes the highest
/// version, whatever order the resolver listed them in.
#[test]
fn highest_version_is_described() {
    let resolver = resolver(IRI, &["1.0.0", "2.0.0", "0.10.3"]);

    let info = do_info(&iri(IRI), &resolver).unwrap();

    assert_eq!(info.version(), "2.0.0");
}

/// Non-semver candidates are skipped.
#[test]
fn non_semver_versions_are_skipped() {
    let resolver = resolver(IRI, &["nightly", "1.0.0"]);

    let info = do_info(&iri(IRI), &resolver).unwrap();

    assert_eq!(info.version(), "1.0.0");
}

#[test]
fn only_non_semver_versions_is_an_error() {
    let resolver = resolver(IRI, &["nightly"]);

    assert_matches!(
        do_info(&iri(IRI), &resolver).unwrap_err(),
        InfoError::NoSemanticVersionsFound(found) if found == vec!["nightly".to_owned()]
    );
}

/// When no candidate can be read, the error says why for each, instead
/// of claiming non-semver versions that were never found.
#[test]
fn no_readable_project_names_the_reasons() {
    let mut missing_meta = project("1.0.0");
    missing_meta.meta = None;
    let resolver = MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(IRI),
        vec![missing_meta, InMemoryProject::default()],
    )]);

    let error = do_info(&iri(IRI), &resolver).unwrap_err();
    let InfoError::NoReadableProject { usage: _, reasons } = &error else {
        panic!("not a read failure: {error:?}");
    };
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert_eq!(
        error.to_string(),
        format!(
            "IRI `{IRI}` resolved to no project that could be read:\n{}",
            reasons
                .iter()
                .map(|reason| format!("        {reason}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    );
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("project is missing")),
        "{reasons:?}"
    );
}

/// A candidate that cannot be read is skipped when another one works.
#[test]
fn an_unreadable_candidate_beside_a_readable_one_is_skipped() {
    let resolver = MemoryResolver::from_iter([(
        Identifier::from_iri_unchecked_str(IRI),
        vec![InMemoryProject::default(), project("1.0.0")],
    )]);

    assert_eq!(do_info(&iri(IRI), &resolver).unwrap().version(), "1.0.0");
}

// --- prerelease versions --------------------------------------------------
//
// `info` considers every version, prereleases included, for every locator
// form. An unconstrained PURL usage ignores pre-releases when resolving, so
// `sysand info` can name a version `sysand add`/`lock` would not pick. This
// matches `cargo`: locking does not choose a pre-release unless requested,
// while `info` shows the latest version, compared as SemVer 2.0 specifies.

/// A non-PURL IRI names one project the user picked out themselves, so its
/// prerelease is described like any other version
#[test]
fn prerelease_only_non_purl_project_is_described() {
    let resolver = resolver(IRI, &["1.0.0-alpha.1"]);

    let info = do_info(&iri(IRI), &resolver).unwrap();

    assert_eq!(info.version(), "1.0.0-alpha.1");
}

/// And a non-PURL IRI with several candidates takes the highest, prerelease
/// included
#[test]
fn non_purl_iri_takes_the_highest_version_including_a_prerelease() {
    let resolver = resolver(IRI, &["2.0.0-beta.1", "1.0.0"]);

    let info = do_info(&iri(IRI), &resolver).unwrap();

    assert_eq!(info.version(), "2.0.0-beta.1");
}

/// A PURL is the only case where solver and info behaviors don't match
#[test]
fn purl_iri_takes_the_highest_version_including_a_prerelease() {
    // Index order: descending, as `versions.json` must be.
    let resolver = resolver(PURL, &["3.0.0-beta.1", "2.0.0", "1.0.0"]);

    let info = do_info(&iri(PURL), &resolver).unwrap();

    assert_eq!(info.version(), "3.0.0-beta.1");
}

#[test]
fn unknown_iri_is_not_found() {
    let resolver = resolver(IRI, &["1.0.0"]);

    assert_matches!(
        do_info(&iri("urn:kpar:other"), &resolver).unwrap_err(),
        InfoError::NotFound {
            usage: _,
            reason: _
        }
    );
}
