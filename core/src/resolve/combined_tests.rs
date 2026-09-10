// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::assert_matches;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use camino::Utf8Path;
use fluent_uri::Iri;
use indexmap::IndexMap;
use typed_path::Utf8UnixPath;

use crate::{
    context::ProjectContext,
    info::{InfoError, do_info},
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectChecksum, ProjectRead, memory::InMemoryProject, utils::Identifier},
    resolve::{
        ResolutionInfo, ResolutionOutcome, ResolveRead,
        combined::{CombinedResolver, NO_RESOLVER},
        memory::{AcceptAll, MemoryResolver},
    },
};

fn minimal_project<S: AsRef<str>, T: AsRef<str>>(name: S, version: T) -> InMemoryProject {
    InMemoryProject {
        info: Some(InterchangeProjectInfoRaw {
            name: name.as_ref().to_owned(),
            publisher: None,
            description: None,
            version: version.as_ref().to_owned(),
            license: None,
            maintainer: vec![],
            website: None,
            topic: vec![],
            usage: vec![],
        }),
        meta: Some(InterchangeProjectMetadataRaw {
            index: IndexMap::new(),
            created: "1970-01-01T00:00:00.000000000Z".to_owned(),
            metamodel: None,
            includes_derived: None,
            includes_implied: None,
            checksum: None,
        }),
        files: HashMap::new(),
        nominal_sources: vec![],
    }
}

#[expect(clippy::unnecessary_wraps)]
fn empty_any_resolver() -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
    Some(MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: HashMap::new(),
    })
}

#[expect(clippy::unnecessary_wraps)]
fn single_project_any_resolver<S: AsRef<str>>(
    uri: S,
    project: InMemoryProject,
) -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
    let uri = Identifier::from_iri_unchecked_str(uri.as_ref());

    let mut projects = HashMap::new();

    projects.insert(uri, vec![project]);

    Some(MemoryResolver {
        iri_predicate: AcceptAll {},
        projects,
    })
}

#[expect(clippy::unnecessary_wraps)]
fn multiple_projects_any_resolver<S: AsRef<str>>(
    uri: S,
    projects: Vec<InMemoryProject>,
) -> Option<MemoryResolver<AcceptAll, InMemoryProject>> {
    let uri = Identifier::from_iri_unchecked_str(uri.as_ref());
    let mut projects_map = HashMap::new();
    projects_map.insert(uri, projects);
    Some(MemoryResolver {
        iri_predicate: AcceptAll {},
        projects: projects_map,
    })
}

fn iri(iri: &str) -> Iri<String> {
    Iri::parse(iri).unwrap().into()
}

#[test]
fn prefer_file_resolver_when_successful() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: single_project_any_resolver(&example_uri, project_a),
        remote_resolver: single_project_any_resolver(&example_uri, project_b.clone()),
        local_resolver: single_project_any_resolver(&example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(&example_uri, project_b),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "a");
}

// TODO: decide and document the resolution policy, in this case: when should
// fallbacks be allowed (relevant for all explicit usages, like http/git/file urls
// and relative paths) and which one should be prioritized? If this test passes
// (by changing CombinedResolver's file resolver NotFound return to abort the resolution),
// then transitive env relative path usages will not get resolved, because
// FileResolver will get the wrong absolute path (due to current env read design,
// base_path will be the env-internal project path, not the original
// source of it), and return NotFound, which will abort the resolution.
// See lock_directory_usage_env_installed_dependency and
// directory_usage_missing_path_falls_back_to_env_resolver
// which conflict with this test
//
// #[test]
// fn prefer_file_resolver_even_when_unresolved() {
//     let example_uri = iri("http://example.com");

//     let project_a = minimal_project("a", "1.2.3");

//     let resolver = CombinedResolver {
//         file_resolver: empty_any_resolver(),
//         remote_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
//         local_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
//         index_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
//     };

//     let xs = do_info(&example_uri, &resolver);
//     assert!(xs.is_err())
// }

#[test]
fn skip_file_resolver_if_unsupported_iri() {
    let example_uri = iri("http://example.com");

    //let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(&example_uri, project_b.clone()),
        local_resolver: single_project_any_resolver(&example_uri, project_b.clone()),
        index_resolver: single_project_any_resolver(&example_uri, project_b),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "b");
}

#[test]
fn prefer_remote_over_index_if_valid_cached() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
        local_resolver: single_project_any_resolver(&example_uri, project_a),
        index_resolver: single_project_any_resolver(&example_uri, project_b),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "a");
}

#[test]
fn prefer_remote_over_index_if_valid_uncached() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");
    let project_c = minimal_project("c", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: single_project_any_resolver(&example_uri, project_a),
        local_resolver: single_project_any_resolver(&example_uri, project_b),
        index_resolver: single_project_any_resolver(&example_uri, project_c),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "b");
}

#[test]
fn skip_remote_if_unsupported_uncached() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: single_project_any_resolver(&example_uri, project_b),
        index_resolver: single_project_any_resolver(&example_uri, project_a),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "b");
}

#[test]
fn skip_remote_if_unsupported_cached() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(&example_uri, project_a),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "a");
}

#[test]
fn skip_remote_if_unresolved_cached() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: empty_any_resolver(),
        local_resolver: single_project_any_resolver(&example_uri, project_a.clone()),
        index_resolver: single_project_any_resolver(&example_uri, project_a),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "a");
}

fn resolve<R: ResolveRead>(
    resolver: &R,
    iri: &str,
) -> Result<ResolutionOutcome<R::ResolvedStorages>, R::Error> {
    let resolve = ResolutionInfo::iri(Iri::parse(iri).unwrap().into());
    resolver.resolve_read(&resolve)
}

#[test]
fn unsupported_iri() {
    let example_uri = "http://example.com";

    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        remote_resolver: NO_RESOLVER,
        local_resolver: NO_RESOLVER,
        index_resolver: NO_RESOLVER,
    };

    let Ok(ResolutionOutcome::UnsupportedUsageType { .. }) = resolve(&resolver, example_uri) else {
        panic!()
    };
}

#[test]
fn unresolved_iri() {
    let example_uri = "http://example.com";

    let resolver = CombinedResolver {
        file_resolver: empty_any_resolver(),
        remote_resolver: empty_any_resolver(),
        local_resolver: empty_any_resolver(),
        index_resolver: empty_any_resolver(),
    };

    let Ok(ResolutionOutcome::NotFound { .. }) = resolve(&resolver, example_uri) else {
        panic!()
    };
}

#[test]
fn skip_non_semantic_versions() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.2.3");
    let project_b = minimal_project("b", "3.2.1.H");

    let resolver = CombinedResolver {
        file_resolver: multiple_projects_any_resolver(&example_uri, vec![project_a, project_b]),
        remote_resolver: empty_any_resolver(),
        local_resolver: empty_any_resolver(),
        index_resolver: empty_any_resolver(),
    };

    let (info, _) = do_info(&example_uri, &resolver).unwrap();

    assert_eq!(info.name, "a");
}

/// A dependency obtained from an environment (or an index/kpar copy) may
/// declare a `Directory` usage whose relative path only exists in its
/// original source tree. When the file resolver cannot find that path on
/// disk, resolution must fall through to the remaining resolvers, which
/// resolve the usage by the identifier derived from its publisher/name —
/// the behaviour established by
/// `solve::pubgrub_tests::directory_usage_env_transitive`. A typed usage
/// that hard-fails here would abort the whole solve even though the project
/// is installed in the environment.
#[cfg(feature = "filesystem")]
#[test]
fn directory_usage_missing_path_falls_back_to_env_resolver() {
    use crate::{
        env::memory::MemoryStorageEnvironment, model::InterchangeProjectUsage,
        project::ProjectRead as _, resolve::env::EnvResolver, resolve::file::FileResolver,
    };

    let mut widget = minimal_project("widget", "1.0.0");
    widget.info.as_mut().unwrap().publisher = Some("acme".to_owned());

    let env = MemoryStorageEnvironment::from([(
        "pkg:sysand/acme/widget".to_owned(),
        "1.0.0".to_owned(),
        widget,
    )]);

    let resolver = CombinedResolver {
        file_resolver: Some(FileResolver {
            sandbox_roots: None,
        }),
        local_resolver: Some(EnvResolver { env }),
        remote_resolver: NO_RESOLVER,
        index_resolver: NO_RESOLVER,
    };

    // The base path exists (as an env-internal project root would), but
    // `../widget` relative to it does not.
    let tmp = camino_tempfile::tempdir().unwrap();
    let base = tmp.path().join("app");
    std::fs::create_dir(&base).unwrap();

    let usage = InterchangeProjectUsage::Directory {
        dir: "../widget".into(),
        publisher: "acme".to_owned(),
        name: "widget".to_owned(),
    };
    let resolution = ResolutionInfo::new(usage, Some(base));

    let projects = match resolver.resolve_read(&resolution).unwrap() {
        ResolutionOutcome::Resolved(projects) => projects,
        ResolutionOutcome::UnsupportedUsageType { reason }
        | ResolutionOutcome::NotFound { reason }
        | ResolutionOutcome::Unresolvable { reason } => {
            panic!("directory usage should have been resolved from the environment: {reason}")
        }
    };

    let projects: Vec<_> = projects.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(projects.len(), 1);
    let info = projects[0].get_info().unwrap().unwrap();
    assert_eq!(info.name, "widget");
    assert_eq!(info.version, "1.0.0");
}

#[test]
fn no_semantic_versions_error() {
    let example_uri = iri("http://example.com");

    let project_a = minimal_project("a", "1.23");
    let project_b = minimal_project("b", "3.2.1.H");

    let resolver = CombinedResolver {
        file_resolver: multiple_projects_any_resolver(&example_uri, vec![project_a, project_b]),
        remote_resolver: empty_any_resolver(),
        local_resolver: empty_any_resolver(),
        index_resolver: empty_any_resolver(),
    };

    let info_meta = do_info(&example_uri, &resolver);

    assert_matches!(info_meta, Err(InfoError::NoSemanticVersionsFound(_)));
}

/// An in-memory project that counts how often its info/meta are read, so a
/// test can assert that a candidate was *not* inspected.
#[derive(Clone, Debug)]
struct CountingProject {
    inner: InMemoryProject,
    reads: Rc<Cell<usize>>,
}

impl ProjectRead for CountingProject {
    type Error = <InMemoryProject as ProjectRead>::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.reads.set(self.reads.get() + 1);
        self.inner.get_project()
    }

    type SourceReader<'a>
        = <InMemoryProject as ProjectRead>::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.inner.read_source(path)
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        self.inner.sources(ctx)
    }

    fn checksum_canonical_variant(&self) -> Result<ProjectChecksum, Self::Error> {
        self.inner.checksum_canonical_variant()
    }

    fn project_root(&self) -> Option<&Utf8Path> {
        self.inner.project_root()
    }
}

fn counting_index_resolver(
    uri: &str,
    versions: &[&str],
) -> (
    Option<MemoryResolver<AcceptAll, CountingProject>>,
    Rc<Cell<usize>>,
) {
    let reads = Rc::new(Cell::new(0));
    let projects = versions
        .iter()
        .map(|v| CountingProject {
            inner: minimal_project("counted", v),
            reads: reads.clone(),
        })
        .collect();
    let mut map = HashMap::new();
    map.insert(Identifier::from_iri_unchecked_str(uri), projects);
    (
        Some(MemoryResolver {
            iri_predicate: AcceptAll {},
            projects: map,
        }),
        reads,
    )
}

#[test]
fn index_candidates_are_not_probed_when_nothing_local_can_match() {
    let uri = "urn:kpar:counted";
    let (index, reads) = counting_index_resolver(uri, &["1.0.0", "2.0.0"]);
    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        local_resolver: empty_any_resolver(),
        remote_resolver: NO_RESOLVER,
        index_resolver: index,
    };

    let outcome = resolver
        .resolve_read(&ResolutionInfo::iri(Iri::parse(uri).unwrap().into()))
        .unwrap();
    let ResolutionOutcome::Resolved(candidates) = outcome else {
        panic!("expected the IRI to resolve");
    };
    assert_eq!(candidates.count(), 2);
    assert_eq!(
        reads.get(),
        0,
        "enumerating candidates must not read their info/meta when no local project awaits a match"
    );
}

#[test]
fn index_candidates_are_probed_when_a_local_copy_may_match() {
    let uri = "urn:kpar:counted";
    let (index, reads) = counting_index_resolver(uri, &["1.0.0"]);
    let resolver = CombinedResolver {
        file_resolver: NO_RESOLVER,
        local_resolver: single_project_any_resolver(uri, minimal_project("counted", "1.0.0")),
        remote_resolver: NO_RESOLVER,
        index_resolver: index,
    };

    let outcome = resolver
        .resolve_read(&ResolutionInfo::iri(Iri::parse(uri).unwrap().into()))
        .unwrap();
    let ResolutionOutcome::Resolved(candidates) = outcome else {
        panic!("expected the IRI to resolve");
    };
    assert_eq!(candidates.count(), 1);
    // One checksum probe: `checksum_canonical_hex` reads the info and the
    // metadata separately, and each goes through `get_project`.
    assert_eq!(
        reads.get(),
        2,
        "the one candidate must be probed exactly once to match the waiting local copy"
    );
}
