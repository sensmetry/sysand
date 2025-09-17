// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::convert::Infallible;

use crate::{project::null::NullProject, resolve::ResolveRead};

#[derive(Debug)]
pub struct NullResolver {}

impl ResolveRead for NullResolver {
    type Error = Infallible;

    type ProjectStorage = NullProject;

    type ResolvedStorages = std::iter::Empty<Result<Self::ProjectStorage, Infallible>>;

    fn resolve_read(
        &self,
        _uri: &fluent_uri::Iri<String>,
    ) -> Result<super::ResolutionOutcome<Self::ResolvedStorages>, Self::Error> {
        Ok(super::ResolutionOutcome::UnsupportedIRIType(
            "null resolver".to_string(),
        ))
    }
}
