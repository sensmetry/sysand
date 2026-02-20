use std::{
    fmt::{self, Debug},
    io::{self, Read},
};

use thiserror::Error;

use crate::{env::utils::ErrorBound, project::ProjectRead, resolve::ResolveRead};

use super::ResolutionOutcome;

/// Resolver that overrides the resolution of some underlying (lower priority)
/// resolver by that of another (higher priority) resolver.
#[derive(Debug)]
pub struct PriorityResolver<Higher, Lower> {
    higher: Higher,
    lower: Lower,
}

impl<Higher, Lower> PriorityResolver<Higher, Lower> {
    pub fn new(higher: Higher, lower: Lower) -> Self {
        PriorityResolver { higher, lower }
    }
}

#[derive(Error, Debug)]
pub enum PriorityError<HigherError: ErrorBound, LowerError: ErrorBound> {
    #[error(transparent)]
    Higher(HigherError),
    #[error(transparent)]
    Lower(LowerError),
}

#[derive(Debug)]
pub enum PriorityProject<HigherProject, LowerProject> {
    HigherProject(HigherProject),
    LowerProject(LowerProject),
}

#[derive(Debug)]
pub enum PriorityReader<HigherReader, LowerReader> {
    HigherReader(HigherReader),
    LowerReader(LowerReader),
}

pub enum PriorityIterator<Higher: ResolveRead, Lower: ResolveRead> {
    HigherIterator(<<Higher as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter),
    LowerIterator(<<Lower as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter),
}

impl<Higher: ResolveRead + Debug, Lower: ResolveRead + Debug> Debug
    for PriorityIterator<Higher, Lower>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HigherIterator(_arg0) => f
                .debug_tuple("HigherIterator")
                .field(&"<iterator>")
                .finish(),
            Self::LowerIterator(_arg0) => {
                f.debug_tuple("LowerIterator").field(&"<iterator>").finish()
            }
        }
    }
}

impl<Higher: ResolveRead, Lower: ResolveRead> Iterator for PriorityIterator<Higher, Lower> {
    type Item = Result<
        PriorityProject<Higher::ProjectStorage, Lower::ProjectStorage>,
        PriorityError<Higher::Error, Lower::Error>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PriorityIterator::HigherIterator(project) => project.next().map(|x| {
                x.map(PriorityProject::HigherProject)
                    .map_err(PriorityError::Higher)
            }),
            PriorityIterator::LowerIterator(project) => project.next().map(|x| {
                x.map(PriorityProject::LowerProject)
                    .map_err(PriorityError::Lower)
            }),
        }
    }
}

impl<HigherReader: Read, LowerReader: Read> Read for PriorityReader<HigherReader, LowerReader> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            PriorityReader::HigherReader(reader) => reader.read(buf),
            PriorityReader::LowerReader(reader) => reader.read(buf),
        }
    }
}

impl<HigherProject: ProjectRead, LowerProject: ProjectRead> ProjectRead
    for PriorityProject<HigherProject, LowerProject>
{
    type Error = PriorityError<HigherProject::Error, LowerProject::Error>;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        match self {
            PriorityProject::HigherProject(project) => {
                project.get_project().map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => {
                project.get_project().map_err(PriorityError::Lower)
            }
        }
    }

    type SourceReader<'a>
        = PriorityReader<HigherProject::SourceReader<'a>, LowerProject::SourceReader<'a>>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => project
                .read_source(path)
                .map(PriorityReader::HigherReader)
                .map_err(PriorityError::Higher),
            PriorityProject::LowerProject(project) => project
                .read_source(path)
                .map(PriorityReader::LowerReader)
                .map_err(PriorityError::Lower),
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            PriorityProject::HigherProject(project) => project.sources(),
            PriorityProject::LowerProject(project) => project.sources(),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            PriorityProject::HigherProject(project) => project.is_definitely_invalid(),
            PriorityProject::LowerProject(project) => project.is_definitely_invalid(),
        }
    }
}

impl<Higher: ResolveRead, Lower: ResolveRead> ResolveRead for PriorityResolver<Higher, Lower> {
    type Error = PriorityError<Higher::Error, Lower::Error>;

    type ProjectStorage = PriorityProject<Higher::ProjectStorage, Lower::ProjectStorage>;

    type ResolvedStorages = PriorityIterator<Higher, Lower>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        match self
            .higher
            .resolve_read(uri)
            .map_err(PriorityError::Higher)?
        {
            ResolutionOutcome::Resolved(resolved) => {
                return Ok(ResolutionOutcome::Resolved(
                    PriorityIterator::HigherIterator(resolved.into_iter()),
                ));
            }
            ResolutionOutcome::UnsupportedIRIType(msg) => {
                log::debug!("higher priority resolver rejected IRI: {msg}")
            }
            ResolutionOutcome::Unresolvable(msg) => {
                log::debug!("higher priority resolver failed to resolve IRI: {msg}")
            }
        };

        Ok(self
            .lower
            .resolve_read(uri)
            .map_err(PriorityError::Lower)?
            .map(|resolved| PriorityIterator::LowerIterator(resolved.into_iter())))
    }
}

#[cfg(test)]
mod tests {
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
}
