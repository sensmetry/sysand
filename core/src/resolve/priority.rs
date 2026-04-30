// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::{
    fmt::{self, Debug},
    io::{self, Read},
};

use thiserror::Error;

use crate::{
    context::ProjectContext,
    env::utils::ErrorBound,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{CanonicalizationError, ProjectRead},
    resolve::ResolveRead,
};

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
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
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

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => {
                project.sources(ctx).map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => {
                project.sources(ctx).map_err(PriorityError::Lower)
            }
        }
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => {
                project.get_info().map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => {
                project.get_info().map_err(PriorityError::Lower)
            }
        }
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => {
                project.get_meta().map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => {
                project.get_meta().map_err(PriorityError::Lower)
            }
        }
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => {
                project.version().map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => {
                project.version().map_err(PriorityError::Lower)
            }
        }
    }

    fn usage(&self) -> Result<Option<Vec<crate::model::InterchangeProjectUsageRaw>>, Self::Error> {
        match self {
            PriorityProject::HigherProject(project) => {
                project.usage().map_err(PriorityError::Higher)
            }
            PriorityProject::LowerProject(project) => project.usage().map_err(PriorityError::Lower),
        }
    }

    fn is_definitely_invalid(&self) -> bool {
        match self {
            PriorityProject::HigherProject(project) => project.is_definitely_invalid(),
            PriorityProject::LowerProject(project) => project.is_definitely_invalid(),
        }
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        match self {
            PriorityProject::HigherProject(project) => project
                .checksum_canonical_hex()
                .map_err(|e| e.map_project_read(PriorityError::Higher)),
            PriorityProject::LowerProject(project) => project
                .checksum_canonical_hex()
                .map_err(|e| e.map_project_read(PriorityError::Lower)),
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
#[path = "./priority_tests.rs"]
mod tests;
