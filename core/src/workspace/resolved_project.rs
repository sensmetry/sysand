// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use crate::model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw};

/// Wraps a `ProjectRead` and overrides `get_project()` to return a pair of
/// already-resolved `(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)`
/// values. All other `ProjectRead` methods delegate to the inner project.
///
/// Used in workspace builds so that `temporary_from_project` (which calls
/// `get_project()`) sees the resolved values rather than raw files that may
/// contain workspace inheritance placeholders.
pub struct ResolvedProject<'a, P> {
    pub inner: &'a P,
    pub info: InterchangeProjectInfoRaw,
    pub meta: InterchangeProjectMetadataRaw,
}

impl<'a, P: crate::project::ProjectRead> crate::project::ProjectRead for ResolvedProject<'a, P> {
    type Error = P::Error;

    fn get_project(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        Ok((Some(self.info.clone()), Some(self.meta.clone())))
    }

    type SourceReader<'b>
        = P::SourceReader<'b>
    where
        Self: 'b;

    fn read_source<Q: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        path: Q,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        self.inner.read_source(path)
    }

    fn sources(
        &self,
        ctx: &crate::context::ProjectContext,
    ) -> Result<Vec<crate::lock::Source>, Self::Error> {
        self.inner.sources(ctx)
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        self.inner.project_root()
    }
}
