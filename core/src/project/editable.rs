// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};

use crate::project::ProjectRead;

#[derive(Debug)]
/// Treat a project type `P` as an "Editable" project. This simply adds
/// a `source` pointing to the nominal path `nominal_path` when
/// this project is in a lockfile.
/// Project's own path cannot be used as `source`, since it may be
/// absolute to allow the project to be read without changing
/// program's dir to workspace root.
pub struct EditableProject<P: GetPath> {
    inner: P,
    /// Must be relative to workspace root
    nominal_path: Utf8PathBuf,
    include_original_sources: bool,
}

pub trait GetPath {
    // TODO: use camino path
    fn get_path(&self) -> impl AsRef<Utf8Path>;
}

impl<P: GetPath> EditableProject<P> {
    pub fn new(nominal_path: Utf8PathBuf, project: P) -> EditableProject<P> {
        debug_assert!(nominal_path.is_relative());
        EditableProject {
            inner: project,
            nominal_path,
            include_original_sources: false,
        }
    }

    pub fn inner(&self) -> &P {
        &self.inner
    }
}

impl<P: ProjectRead + GetPath> ProjectRead for EditableProject<P> {
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
                editable: self.nominal_path.as_str().into(),
            },
        );

        inner_sources
    }
}
