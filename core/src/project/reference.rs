// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectFormat, ProjectRead},
};

// Arc wrapper around project to make cloning possible
// (necessary for compatibility with `MemoryResolver`)
#[derive(Debug)]
pub struct ProjectReference<Project: ProjectRead> {
    project: Arc<Project>,
}

impl<Project: ProjectRead> ProjectReference<Project> {
    pub fn new(project: Project) -> Self {
        Self {
            project: Arc::new(project),
        }
    }
}

impl<Project: ProjectRead> Clone for ProjectReference<Project> {
    fn clone(&self) -> Self {
        Self {
            project: self.project.clone(),
        }
    }
}

impl<Project: ProjectRead> ProjectRead for ProjectReference<Project> {
    /// Forwarded, not defaulted: this type wraps another project, and
    /// the default would claim json for a `model.toml` project it holds.
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        self.project.format()
    }

    type Error = Project::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.project.get_project()
    }

    type SourceReader<'a>
        = Project::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.project.read_source(path)
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        self.project.sources(ctx)
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        self.project.source_may_offer_multiple_versions()
    }

    fn checksum_canonical_variant(&self) -> Result<super::ProjectChecksum, Self::Error> {
        self.project.checksum_canonical_variant()
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        self.project.project_root()
    }
}

#[cfg(feature = "filesystem")]
#[cfg(test)]
#[path = "./reference_tests.rs"]
mod tests;
