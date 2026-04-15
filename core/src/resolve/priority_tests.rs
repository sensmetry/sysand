// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use fluent_uri::Iri;
use indexmap::IndexMap;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectRead as _, memory::InMemoryProject},
    resolve::{
        ResolutionOutcome, ResolveRead,
        memory::{AcceptAll, MemoryResolver},
    },
};

fn mock_project<S: AsRef<str>, T: AsRef<str>, V: AsRef<str>>(
    uri: S,
    name: T,
    version: V,
) -> (Iri<String>, InMemoryProject) {
    (
        Iri::parse(uri.as_ref().to_string()).unwrap(),
        InMemoryProject {
            info: Some(InterchangeProjectInfoRaw {
                name: name.as_ref().to_string(),
                publisher: None,
                description: None,
                version: version.as_ref().to_string(),
                license: None,
                maintainer: vec![],
                website: None,
                topic: vec![],
                usage: vec![],
            }),
            meta: Some(InterchangeProjectMetadataRaw {
                index: IndexMap::default(),
                created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                metamodel: None,
                includes_derived: None,
                includes_implied: None,
                checksum: Some(IndexMap::default()),
            }),
            files: HashMap::default(),
            nominal_sources: vec![],
        },
    )
}

fn mock_resolver<I: IntoIterator<Item = (Iri<String>, InMemoryProject)>>(
    projects: I,
) -> MemoryResolver<AcceptAll, InMemoryProject> {
    MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::from_iter(projects.into_iter().map(|(k, v)| (k, vec![v]))),
    }
}

fn expect_to_resolve<R: ResolveRead, S: AsRef<str>>(
    resolver: &R,
    uri: S,
) -> Vec<R::ProjectStorage> {
    let resolved = resolver.resolve_read_raw(uri).unwrap();

    let foo_projects: Result<Vec<_>, _> =
        if let ResolutionOutcome::Resolved(foo_projects) = resolved {
            foo_projects.into_iter().collect()
        } else {
            panic!("expected foo to resolve")
        };

    foo_projects.unwrap()
}

#[test]
fn resolution_priority() -> Result<(), Box<dyn std::error::Error>> {
    let higher = mock_resolver([
        mock_project("urn:kpar:foo", "foo", "1.2.3"),
        mock_project("urn:kpar:bar", "bar", "1.2.3"),
    ]);

    let lower = mock_resolver([
        mock_project("urn:kpar:bar", "bar", "3.2.1"),
        mock_project("urn:kpar:baz", "baz", "3.2.1"),
    ]);

    let resolver = super::PriorityResolver::new(higher, lower);

    let foos = expect_to_resolve(&resolver, "urn:kpar:foo");

    assert_eq!(foos.len(), 1);
    assert_eq!(foos[0].version().unwrap(), Some("1.2.3".to_string()));

    let bars = expect_to_resolve(&resolver, "urn:kpar:bar");

    assert_eq!(bars.len(), 1);
    assert_eq!(bars[0].version().unwrap(), Some("1.2.3".to_string()));

    let bazs = expect_to_resolve(&resolver, "urn:kpar:baz");

    assert_eq!(bazs.len(), 1);
    assert_eq!(bazs[0].version().unwrap(), Some("3.2.1".to_string()));

    Ok(())
}
