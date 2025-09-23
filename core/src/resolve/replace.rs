use std::io::Read;

use thiserror::Error;

use crate::{project::ProjectRead, resolve::ResolveRead};

/// Resolver that overrides the resolution of some underlying (secondary)
/// by that of another (primary) resolver.
#[derive(Debug)]
pub struct ReplaceResolver<Primary, Secondary> {
    primary: Primary,
    secondary: Secondary,
}

impl<Primary, Secondary> ReplaceResolver<Primary, Secondary> {
    pub fn new(primary: Primary, secondary: Secondary) -> Self {
        ReplaceResolver { primary, secondary }
    }
}

#[derive(Error, Debug)]
pub enum ReplaceError<PrimaryError, SecondaryError> {
    #[error(transparent)]
    PrimaryError(PrimaryError),
    #[error(transparent)]
    SecondaryError(SecondaryError),
}

#[derive(Debug)]
pub enum ReplaceProject<PrimaryProject, SecondaryProject> {
    PrimaryProject(PrimaryProject),
    SecondaryProject(SecondaryProject),
}

#[derive(Debug)]
pub enum ReplaceReader<PrimaryReader, SecondaryReader> {
    PrimaryReader(PrimaryReader),
    SecondaryReader(SecondaryReader),
}

pub enum ReplaceIterator<Primary: ResolveRead, Secondary: ResolveRead> {
    PrimaryIterator(<<Primary as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter),
    SecondaryIterator(<<Secondary as ResolveRead>::ResolvedStorages as IntoIterator>::IntoIter),
}

impl<Primary: ResolveRead + std::fmt::Debug, Secondary: ResolveRead + std::fmt::Debug>
    std::fmt::Debug for ReplaceIterator<Primary, Secondary>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrimaryIterator(_arg0) => f
                .debug_tuple("PrimaryIterator")
                .field(&"<iterator>")
                .finish(),
            Self::SecondaryIterator(_arg0) => f
                .debug_tuple("SecondaryIterator")
                .field(&"<iterator>")
                .finish(),
        }
    }
}

impl<Primary: ResolveRead, Secondary: ResolveRead> Iterator
    for ReplaceIterator<Primary, Secondary>
{
    type Item = Result<
        ReplaceProject<Primary::ProjectStorage, Secondary::ProjectStorage>,
        ReplaceError<Primary::Error, Secondary::Error>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ReplaceIterator::PrimaryIterator(project) => project.next().map(|x| {
                x.map(ReplaceProject::PrimaryProject)
                    .map_err(ReplaceError::PrimaryError)
            }),
            ReplaceIterator::SecondaryIterator(project) => project.next().map(|x| {
                x.map(ReplaceProject::SecondaryProject)
                    .map_err(ReplaceError::SecondaryError)
            }),
        }
    }
}

impl<PrimaryReader: Read, SecondaryReader: Read> Read
    for ReplaceReader<PrimaryReader, SecondaryReader>
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ReplaceReader::PrimaryReader(reader) => reader.read(buf),
            ReplaceReader::SecondaryReader(reader) => reader.read(buf),
        }
    }
}

impl<PrimaryProject: ProjectRead, SecondaryProject: ProjectRead> ProjectRead
    for ReplaceProject<PrimaryProject, SecondaryProject>
{
    type Error = ReplaceError<PrimaryProject::Error, SecondaryProject::Error>;

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
            ReplaceProject::PrimaryProject(project) => {
                project.get_project().map_err(ReplaceError::PrimaryError)
            }
            ReplaceProject::SecondaryProject(project) => {
                project.get_project().map_err(ReplaceError::SecondaryError)
            }
        }
    }

    type SourceReader<'a>
        = ReplaceReader<PrimaryProject::SourceReader<'a>, SecondaryProject::SourceReader<'a>>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        match self {
            ReplaceProject::PrimaryProject(project) => project
                .read_source(path)
                .map(ReplaceReader::PrimaryReader)
                .map_err(ReplaceError::PrimaryError),
            ReplaceProject::SecondaryProject(project) => project
                .read_source(path)
                .map(ReplaceReader::SecondaryReader)
                .map_err(ReplaceError::SecondaryError),
        }
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        match self {
            ReplaceProject::PrimaryProject(project) => project.sources(),
            ReplaceProject::SecondaryProject(project) => project.sources(),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            ReplaceProject::PrimaryProject(project) => project.is_definitely_invalid(),
            ReplaceProject::SecondaryProject(project) => project.is_definitely_invalid(),
        }
    }
}

impl<Primary: ResolveRead, Secondary: ResolveRead> ResolveRead
    for ReplaceResolver<Primary, Secondary>
{
    type Error = ReplaceError<Primary::Error, Secondary::Error>;

    type ProjectStorage = ReplaceProject<Primary::ProjectStorage, Secondary::ProjectStorage>;

    type ResolvedStorages = ReplaceIterator<Primary, Secondary>;

    fn resolve_read(
        &self,
        uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        if let super::ResolutionOutcome::Resolved(resolved) = self
            .primary
            .resolve_read(uri)
            .map_err(ReplaceError::PrimaryError)?
        {
            return Ok(super::ResolutionOutcome::Resolved(
                ReplaceIterator::PrimaryIterator(resolved.into_iter()),
            ));
        };

        Ok(self
            .secondary
            .resolve_read(uri)
            .map_err(ReplaceError::SecondaryError)?
            .map(|resolved| ReplaceIterator::SecondaryIterator(resolved.into_iter())))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
    fn resolution_priority() -> Result<(), Box<dyn std::error::Error>> {
        let primary = mock_resolver([
            mock_project("urn::kpar::foo", "foo", "1.2.3"),
            mock_project("urn::kpar::bar", "bar", "1.2.3"),
        ]);

        let secondary = mock_resolver([
            mock_project("urn::kpar::bar", "bar", "3.2.1"),
            mock_project("urn::kpar::baz", "baz", "3.2.1"),
        ]);

        let resolver = super::ReplaceResolver::new(primary, secondary);

        let foos = expect_to_resolve(&resolver, "urn::kpar::foo");

        assert_eq!(foos.len(), 1);
        assert_eq!(foos[0].version().unwrap(), Some("1.2.3".to_string()));

        let bars = expect_to_resolve(&resolver, "urn::kpar::bar");

        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].version().unwrap(), Some("1.2.3".to_string()));

        let bazs = expect_to_resolve(&resolver, "urn::kpar::baz");

        assert_eq!(bazs.len(), 1);
        assert_eq!(bazs[0].version().unwrap(), Some("3.2.1".to_string()));

        Ok(())
    }
}
