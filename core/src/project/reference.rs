// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use crate::project::ProjectRead;

// Arc wrapper around project to make cloning possible
// (necessary for compatibility with ReadEnvironment)
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
    type Error = Project::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
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

    fn sources(&self) -> Vec<crate::lock::Source> {
        self.project.sources()
    }
}

#[cfg(feature = "filesystem")]
#[cfg(test)]
mod test {
    use crate::project::{local_kpar::LocalKParProject, reference::ProjectReference};
    #[test]
    fn test_kpar() {
        let kpar = ProjectReference::new(LocalKParProject::new("path", "root").unwrap());
        let _clone = kpar.clone();
    }
}
