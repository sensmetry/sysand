// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::Utf8Path;
use thiserror::Error;

use crate::{
    env::utils::ErrorBound,
    model::{
        InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw, InterchangeProjectUsage,
        InterchangeProjectUsageRaw, InterchangeProjectValidationError,
    },
    project::ProjectRead,
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Error, Debug)]
pub enum InfoError<Error: ErrorBound> {
    #[error("cannot resolve usage: {0}")]
    Unresolvable(String),
    #[error("usage {0} is not supported: {1}")]
    UnsupportedUsageType(InterchangeProjectUsage, String),
    #[error("usage {0} is invalid: {1}")]
    InvalidUsage(
        InterchangeProjectUsageRaw,
        InterchangeProjectValidationError,
    ),
    #[error("usage {0} was not found: {1}")]
    NotFound(InterchangeProjectUsage, String),
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
        Ok((Some(_), None)) => {
            log::warn!(
                "ignoring a partial project, it has info but not metadata",
                //uri.as_ref()
            );
            None
        }
        Ok((None, Some(_))) => {
            log::warn!(
                "ignoring a partial project, it has metadata but not info",
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
    usage: &InterchangeProjectUsageRaw,
    base_path: Option<impl AsRef<Utf8Path>>,
    resolver: &R,
) -> Result<Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>, InfoError<R::Error>> {
    let outcome = resolver.resolve_read_raw(usage, base_path)?;

    match outcome {
        ResolutionOutcome::Resolved(resolved) => {
            let mut it = resolved.into_iter().peekable();
            assert!(it.peek().is_some());

            let mut result = vec![];
            for alt in it {
                let candidate_project = match alt {
                    Ok(cp) => cp,
                    Err(e) => {
                        // These errors may be ugly, as `resolved` includes all
                        // possible candidates, with expectation that only some
                        // of them will work. So we don't show these by default
                        log::debug!("skipping candidate project: {e}");
                        continue;
                    }
                };
                if let Some(info_meta) = do_info_project(&candidate_project) {
                    result.push(info_meta);
                };
            }
            Ok(result)
        }
        ResolutionOutcome::UnsupportedUsageType { usage, reason } => {
            Err(InfoError::UnsupportedUsageType(usage, reason))
        }
        ResolutionOutcome::NotFound(usage, reason) => Err(InfoError::NotFound(usage, reason)),
        ResolutionOutcome::InvalidUsage(usage, reason) => {
            Err(InfoError::InvalidUsage(usage, reason))
        }
        ResolutionOutcome::Unresolvable(msg) => Err(InfoError::Unresolvable(msg)),
    }
}
