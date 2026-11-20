// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

#[cfg(feature = "filesystem")]
use crate::workspace::model_toml::Workspace;
use crate::{
    project::null::NullProject,
    resolve::{ResolutionInfo, ResolveRead},
};

#[derive(Debug, Clone, Copy)]
pub struct NullResolver;

impl ResolveRead for NullResolver {
    type Error = !;

    #[cfg(feature = "filesystem")]
    fn within_workspace(self, _workspace: std::sync::Arc<Workspace>) -> Self {
        // This resolver reads no local targets.
        self
    }

    type ProjectStorage = NullProject;

    type ResolvedStorages = std::iter::Empty<Result<Self::ProjectStorage, !>>;

    fn resolve_read(
        &self,
        _resolve: &ResolutionInfo,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(super::ResolutionOutcome::UnsupportedUsageType {
            reason: String::from("null resolver"),
        })
    }
}
