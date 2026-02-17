// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::project::{ProjectRead, Utf8UnixPath};

/// Pair of project storages where `local` and `remote` contain the same project
/// content, while `local` is easier and faster to access. The CachedProject is
/// to be used in place of `remote` (in particular they return the same sources,
/// unlike `local`) but faster access.
#[derive(Clone, Debug)]
pub struct CachedProject<Local: ProjectRead, Remote: ProjectRead> {
    local: Local,
    remote: Remote,
}

impl<Local: ProjectRead, Remote: ProjectRead> CachedProject<Local, Remote> {
    /// Create a new CachedProject. Assume that `local` is a cached version of remote.
    pub fn new(local: Local, remote: Remote) -> Self {
        CachedProject::<Local, Remote> { local, remote }
    }
}

impl<Local: ProjectRead, Remote: ProjectRead> ProjectRead for CachedProject<Local, Remote> {
    type Error = Local::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<crate::model::InterchangeProjectInfoRaw>,
            Option<crate::model::InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        self.local.get_project()
    }

    type SourceReader<'a>
        = Local::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.local.read_source(path)
    }

    fn sources(&self) -> Vec<crate::lock::Source> {
        self.remote.sources()
    }

    fn is_definitely_invalid(&self) -> bool {
        self.local.is_definitely_invalid()
    }
}
