// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::{
    context::ProjectContext,
    lock::Source,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{CanonicalizationError, ProjectRead, Utf8UnixPath},
};

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
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
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

    /// It is assumed here that `remote.sources()` is infallible
    // Can't return error, since return type is local project error, but we call
    // remote project sources
    // TODO: more elegant solution
    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        Ok(self.remote.sources(ctx).unwrap())
    }

    fn get_info(&self) -> Result<Option<InterchangeProjectInfoRaw>, Self::Error> {
        self.local.get_info()
    }

    fn get_meta(&self) -> Result<Option<InterchangeProjectMetadataRaw>, Self::Error> {
        self.local.get_meta()
    }

    fn version(&self) -> Result<Option<String>, Self::Error> {
        self.local.version()
    }

    fn usage(&self) -> Result<Option<Vec<crate::model::InterchangeProjectUsageRaw>>, Self::Error> {
        self.local.usage()
    }

    fn is_definitely_invalid(&self) -> bool {
        self.local.is_definitely_invalid()
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        // Delegate to `local`: the cached archive is the authoritative
        // content and any checksum it produces matches the remote's.
        // Delegating through the default would short-circuit back to
        // `self.get_project` (also local) but without picking up any
        // override a leaf might install.
        self.local.checksum_canonical_hex()
    }
}
