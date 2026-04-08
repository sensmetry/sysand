// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::convert::Infallible;

use camino::Utf8Path;

use crate::{model::InterchangeProjectUsage, project::null::NullProject, resolve::ResolveRead};

#[derive(Debug)]
pub struct NullResolver {}

impl ResolveRead for NullResolver {
    type Error = Infallible;

    type ProjectStorage = NullProject;

    type ResolvedStorages = std::iter::Empty<Result<Self::ProjectStorage, Infallible>>;

    fn resolve_read(
        &self,
        usage: &InterchangeProjectUsage,
        _base_path: Option<impl AsRef<Utf8Path>>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(super::ResolutionOutcome::UnsupportedUsageType {
            usage: usage.to_owned(),
            reason: "null resolver".to_string(),
        })
    }
}
