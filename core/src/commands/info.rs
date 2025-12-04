// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectRead,
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Error, Debug)]
pub enum InfoError<Error: std::error::Error> {
    #[error("failed to resolve {0}")]
    NoResolve(String),
    #[error("failure during resolution: {0}")]
    Resolution(#[from] Error),
}

pub fn do_info_project<P: ProjectRead>(
    project: &P,
) -> Option<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> {
    match project.get_project() {
        Ok((Some(info), Some(meta))) => Some((info, meta)),
        Ok((None, None)) => {
            // TODO: Add URI hints to projects and use them here
            log::warn!(
                "ignoring a missing project",
                //uri.as_ref()
            );
            None
        }
        Ok(_) => {
            log::warn!(
                "ignoring a partial project",
                //uri.as_ref()
            );
            None
        }
        Err(e) => {
            log::warn!(
                "ignoring an invalid project: {e}",
                //uri.as_ref()
            );
            None
        }
    }
}

pub fn do_info<S: AsRef<str>, R: ResolveRead>(
    uri: S,
    resolver: &R,
) -> Result<Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>, InfoError<R::Error>> {
    let outcome = resolver.resolve_read_raw(uri.as_ref())?;

    match outcome {
        ResolutionOutcome::Resolved(resolved) => {
            let mut result: Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)> =
                vec![];

            for project in resolved.into_iter() {
                if let Some(info_meta) = do_info_project(&project?) {
                    result.push(info_meta);
                }
            }

            Ok(result)
        }
        ResolutionOutcome::UnsupportedIRIType(e) => Err(InfoError::NoResolve(format!(
            "unsupported IRI '{}': {}",
            uri.as_ref(),
            e
        ))),
        ResolutionOutcome::Unresolvable(e) => Err(InfoError::NoResolve(format!(
            "IRI '{}': {}",
            uri.as_ref(),
            e
        ))),
    }
}
