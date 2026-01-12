// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use thiserror::Error;

use crate::{
    env::utils::ErrorBound,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectRead,
    resolve::{ResolutionOutcome, ResolveRead},
};

#[derive(Error, Debug)]
pub enum InfoError<Error: ErrorBound> {
    #[error("failed to resolve IRI `{0}`: {1}")]
    NoResolve(Box<str>, String),
    #[error("IRI `{0}` is not supported: {1}")]
    UnsupportedIri(Box<str>, String),
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
    uri: S,
    resolver: &R,
) -> Result<Vec<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw)>, InfoError<R::Error>> {
    let outcome = resolver.resolve_read_raw(uri.as_ref())?;

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
        ResolutionOutcome::UnsupportedIRIType(e) => {
            Err(InfoError::UnsupportedIri(uri.as_ref().into(), e))
        }
        ResolutionOutcome::Unresolvable(e) => Err(InfoError::NoResolve(uri.as_ref().into(), e)),
    }
}
