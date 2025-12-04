// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::project::ProjectRead;

#[derive(Debug)]
/// Treat a project type `P` as an "Editable" project. This simply adds
/// a `source` pointing to the nominal path `nominal_path` when
/// this project is in a lockfile.
pub struct EditableProject<P> {
    inner: P,
    nominal_path: String,
    include_original_sources: bool,
}

impl<P> EditableProject<P> {
    pub fn new<Q: AsRef<str>>(nominal_path: Q, project: P) -> EditableProject<P> {
        EditableProject {
            inner: project,
            nominal_path: nominal_path.as_ref().to_string(),
            include_original_sources: false,
        }
    }
}

impl<P: ProjectRead> ProjectRead for EditableProject<P> {
    type Error = P::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.inner.get_project()
    }

    type SourceReader<'a>
        = P::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<Q: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: Q,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.inner.read_source(path)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        let mut inner_sources = if self.include_original_sources {
            self.inner.sources()
        } else {
            vec![]
        };

        inner_sources.insert(
            0,
            crate::lock::Source::Editable {
                editable: self.nominal_path.clone(),
            },
        );

        inner_sources
    }
}
