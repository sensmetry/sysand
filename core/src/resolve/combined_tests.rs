// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::collections::HashMap;

use fluent_uri::Iri;
use indexmap::IndexMap;

use crate::{
    info::do_info,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::memory::InMemoryProject,
    resolve::{
        ResolveRead,
        combined::{CombinedResolver, NO_RESOLVER},
        memory::{AcceptAll, MemoryResolver},
    },
};

fn minimal_project<S: AsRef<str>, T: AsRef<str>>(name: S, version: T) -> InMemoryProject {
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
            index: IndexMap::new(),
            created: "1970-01-01T00:00:00.000000000Z".to_string(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }),
        files: HashMap::new(),
        nominal_sources: vec![],
    }
}

// const SCHEME_FILE: &Scheme = Scheme::new_or_panic("file");

fn empty_any_resolver() -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
    Some(MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::new(),
    })
}

fn single_project_any_resolver<S: AsRef<str>>(
    uri: S,
    project: InMemoryProject,
) -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
    let uri = Iri::parse(uri.as_ref().to_string()).unwrap();

    let mut projects = HashMap::new();

    projects.insert(uri, vec![project]);

    Some(MemoryResolver {
        iri_predicate: AcceptAll {},
        projects,
    })
}

// fn single_project_file_resolver<S: AsRef<str>>(
//     uri: S,
//     project: ProjectMemoryStorage,
// ) -> MemoryResolver<AcceptScheme<'static>, ProjectMemoryStorage> {
//     let uri = fluent_uri::Iri::parse(uri.as_ref().to_string()).unwrap();

//     if uri.scheme() != SCHEME_FILE {
//         panic!("Invalid IRI for file resolver");
//     }

//     let mut projects = HashMap::new();

//     projects.insert(uri, project);

//     MemoryResolver {
//         iri_predicate: AcceptScheme {
//             scheme: SCHEME_FILE,
//         },
//         projects: projects,
//     }
// }

#[test]
fn prefer_file_resolver_when_successful() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        remote_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 1);
    assert_eq!(xs[0].0.name, "a");
}

#[test]
fn prefer_file_resolver_even_when_unresolved() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");

    let resolver = CombinedResolver {
        file_resolver: empty_any_resolver(),
        remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
    };

    let xs = do_info(example_uri, &resolver);

    assert!(xs.is_err())
}

#[test]
fn skip_file_resolver_if_unsupported_iri() {
    let example_uri = "http://example.com";

    //let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 1);
    assert_eq!(xs[0].0.name, "b");
}

#[test]
fn prefer_remote_over_index_if_valid_cached() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_b.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 1);
    assert_eq!(xs[0].0.name, "a");
}

#[test]
fn prefer_remote_over_index_if_valid_uncached() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");
    let project_c = minimal_project("c", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_c.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 2);
    assert_eq!(xs[0].0.name, "a");
    assert_eq!(xs[1].0.name, "b");
}

#[test]
fn skip_remote_if_unsupported_uncached() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: single_project_any_resolver(example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 2);
    assert_eq!(xs[0].0.name, "a");
    assert_eq!(xs[1].0.name, "b");
}

#[test]
fn skip_remote_if_unsupported_cached() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 1);
    assert_eq!(xs[0].0.name, "a");
}

#[test]
fn skip_remote_if_unresolved_cached() {
    let example_uri = "http://example.com";

    let project_a = minimal_project("a", "1.2.3");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: empty_any_resolver(),
        local_resolver: single_project_any_resolver(example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(example_uri, project_a.clone()),
    };

    let xs = do_info(example_uri, &resolver).unwrap();

    assert_eq!(xs.len(), 1);
    assert_eq!(xs[0].0.name, "a");
}

#[test]
fn unsupported_iri_test() {
    let example_uri = "http://example.com";

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: NO_RESOLVER,
        index_resolver: NO_RESOLVER,
    };

    let Ok(crate::resolve::ResolutionOutcome::UnsupportedIRIType(_)) =
        resolver.resolve_read_raw(example_uri)
    else {
        panic!()
    };
}

#[test]
fn unresolved_iri_test() {
    let example_uri = "http://example.com";

    let resolver = CombinedResolver {
        file_resolver: empty_any_resolver(),
        remote_resolver: empty_any_resolver(),
        local_resolver: empty_any_resolver(),
        index_resolver: empty_any_resolver(),
    };

    let Ok(crate::resolve::ResolutionOutcome::Unresolvable(_)) =
        resolver.resolve_read_raw(example_uri)
    else {
        panic!()
    };
}
