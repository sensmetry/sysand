use std::iter::Flatten;

use crate::resolve::ResolveRead;

/// Takes a sequence of similar resolvers, and tries them in sequence.
/// First resolves all versions in the first environment, then all
/// in the second, ...
#[derive(Debug)]
pub struct SequentialResolve<R: ResolveRead> {
    inner: Vec<R>,
}

impl<R: ResolveRead> SequentialResolve<R> {
    pub fn new<I: IntoIterator<Item = R>>(resolvers: I) -> Self {
        SequentialResolve {
            inner: resolvers.into_iter().collect(),
        }
    }
}

impl<R: ResolveRead> ResolveRead for SequentialResolve<R> {
    type Error = R::Error;

    type ProjectStorage = R::ProjectStorage;

    type ResolvedStorages = Flatten<std::vec::IntoIter<<R as ResolveRead>::ResolvedStorages>>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        let mut iters = vec![];
        let mut any_supported = false;
        let mut msgs = vec![];

        for resolver in &self.inner {
            match resolver.resolve_read(uri)? {
                crate::resolve::ResolutionOutcome::Resolved(storages) => {
                    any_supported = true;
                    iters.push(storages)
                }
                crate::resolve::ResolutionOutcome::UnsupportedIRIType(msg) => {
                    msgs.push(msg);
                }
                crate::resolve::ResolutionOutcome::Unresolvable(msg) => {
                    any_supported = true;
                    msgs.push(msg);
                }
            }
        }

        if !iters.is_empty() {
            Ok(crate::resolve::ResolutionOutcome::Resolved(
                iters.into_iter().flatten(),
            ))
        } else if any_supported {
            Ok(crate::resolve::ResolutionOutcome::Unresolvable(format!(
                "Unresolvable: {:?}",
                msgs
            )))
        } else {
            Ok(crate::resolve::ResolutionOutcome::UnsupportedIRIType(
                format!("Unsupported IRI: {:?}", msgs),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use indexmap::IndexMap;

    use crate::{
        model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
        project::{ProjectRead, memory::InMemoryProject},
        resolve::{
            ResolutionOutcome, ResolveRead,
            memory::{AcceptAll, MemoryResolver},
            sequential::SequentialResolve,
        },
    };

    fn mock_project<S: AsRef<str>, T: AsRef<str>, V: AsRef<str>>(
        uri: S,
        name: T,
        version: V,
    ) -> (fluent_uri::Iri<String>, InMemoryProject) {
        (
            fluent_uri::Iri::parse(uri.as_ref().to_string()).unwrap(),
            InMemoryProject {
                info: Some(InterchangeProjectInfoRaw {
                    name: name.as_ref().to_string(),
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
                    created: chrono::Utc::now().to_rfc3339(),
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

    fn mock_resolver<I: IntoIterator<Item = (fluent_uri::Iri<String>, InMemoryProject)>>(
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
    fn test_resolution_preference() -> Result<(), Box<dyn std::error::Error>> {
        let resolver_1 = mock_resolver([
            mock_project("urn::kpar::foo", "foo", "1.2.3"),
            mock_project("urn::kpar::bar", "bar", "1.2.3"),
        ]);

        let resolver_2 = mock_resolver([
            mock_project("urn::kpar::bar", "bar", "3.2.1"),
            mock_project("urn::kpar::baz", "baz", "3.2.1"),
        ]);

        let resolver = SequentialResolve::new([resolver_1, resolver_2]);

        let foos = expect_to_resolve(&resolver, "urn::kpar::foo");

        assert_eq!(foos.len(), 1);
        assert_eq!(foos[0].version().unwrap(), Some("1.2.3".to_string()));

        let bars = expect_to_resolve(&resolver, "urn::kpar::bar");

        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].version().unwrap(), Some("1.2.3".to_string()));
        assert_eq!(bars[1].version().unwrap(), Some("3.2.1".to_string()));

        let bazs = expect_to_resolve(&resolver, "urn::kpar::baz");

        assert_eq!(bazs.len(), 1);
        assert_eq!(bazs[0].version().unwrap(), Some("3.2.1".to_string()));

        Ok(())
    }
}
