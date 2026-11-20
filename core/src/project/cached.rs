// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::project::VersionAndUsage;
use crate::{
    context::ProjectContext,
    lock::Source,
    manifest::edit::DocumentMut,
    project::{
        CanonicalizationError, ManifestError, ProjectDefinition, ProjectFormat, ProjectRead,
        Utf8UnixPath,
    },
};

/// Pair of project storages where `local` and `remote` contain the same project
/// content, while `local` is easier and faster to access. A `CachedProject`
/// stands in for `remote`: it reports the same sources (unlike `local`) but
/// reads faster.
#[derive(Clone, Debug)]
pub struct CachedProject<Local: ProjectRead, Remote: ProjectRead> {
    local: Local,
    remote: Remote,
}

impl<Local: ProjectRead, Remote: ProjectRead> CachedProject<Local, Remote> {
    /// Create a new CachedProject. Assume that `local` is a cached version of remote.
    pub fn new(local: Local, remote: Remote) -> Self {
        Self { local, remote }
    }
}

/// A read of the local copy failed, or a read of the remote, which only
/// [`ProjectRead::sources`] does.
#[derive(Error, Debug)]
pub enum CachedProjectError<LocalError, RemoteError> {
    #[error(transparent)]
    Local(LocalError),
    #[error(transparent)]
    Remote(RemoteError),
}

impl<Local: ProjectRead, Remote: ProjectRead> ProjectRead for CachedProject<Local, Remote> {
    fn format(&self) -> Result<ProjectFormat, Self::Error> {
        self.local.format().map_err(CachedProjectError::Local)
    }

    fn manifest_document(&self) -> Result<DocumentMut, ManifestError<Self::Error>> {
        self.local
            .manifest_document()
            .map_err(|error| error.map_project_read(CachedProjectError::Local))
    }

    type Error = CachedProjectError<Local::Error, Remote::Error>;

    fn get_project(&self) -> Result<Option<ProjectDefinition>, Self::Error> {
        self.local.get_project().map_err(CachedProjectError::Local)
    }

    type SourceReader<'a>
        = Local::SourceReader<'a>
    where
        Self: 'a;

    fn read_source<P: AsRef<Utf8UnixPath>>(
        &self,
        path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.local
            .read_source(path)
            .map_err(CachedProjectError::Local)
    }

    fn sources(&self, ctx: &ProjectContext) -> Result<Vec<Source>, Self::Error> {
        self.remote.sources(ctx).map_err(CachedProjectError::Remote)
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        // `remote` is the authoritative source; `local` is only a faster copy of it.
        self.remote.source_may_offer_multiple_versions()
    }

    fn version_and_usage(&self) -> Result<Option<VersionAndUsage>, Self::Error> {
        self.local
            .version_and_usage()
            .map_err(CachedProjectError::Local)
    }

    fn is_definitely_invalid(&self) -> bool {
        self.local.is_definitely_invalid()
    }

    fn checksum_canonical_hex(&self) -> Result<Option<String>, CanonicalizationError<Self::Error>> {
        // Delegate to `local`, whose checksum matches the remote's, so a
        // leaf's own override is picked up.
        self.local
            .checksum_canonical_hex()
            .map_err(|error| error.map_project_read(CachedProjectError::Local))
    }

    fn checksum_canonical_variant(&self) -> Result<super::ProjectChecksum, Self::Error> {
        self.local
            .checksum_canonical_variant()
            .map_err(CachedProjectError::Local)
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        self.remote.project_root()
    }
}
