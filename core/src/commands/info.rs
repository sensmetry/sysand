// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use semver::Version;
use thiserror::Error;

use crate::{
    env::utils::ErrorBound,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::ProjectRead,
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::format_err,
};

#[derive(Error, Debug)]
pub enum InfoProjectError<Error: ErrorBound> {
    // TODO: Add URI hints to projects and use them here
    #[error("project is missing")]
    MissingProject,
    #[error("project has .meta.json but not .project.json")]
    MissingInfo,
    #[error("project has .project.json but not .meta.json")]
    MissingMeta,
    #[error(transparent)]
    InvalidProject(#[from] Error),
}

pub fn do_info_project<P: ProjectRead>(
    project: &P,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), InfoProjectError<P::Error>>
{
    match project.get_project() {
        Ok((Some(info), Some(meta))) => Ok((info, meta)),
        Ok((None, None)) => Err(InfoProjectError::MissingProject),
        Ok((None, Some(_))) => Err(InfoProjectError::MissingInfo),
        Ok((Some(_), None)) => Err(InfoProjectError::MissingMeta),
        Err(err) => Err(InfoProjectError::InvalidProject(err)),
    }
}

#[derive(Error, Debug)]
pub enum InfoError<Error: ErrorBound> {
    #[error("none of the following found versions are valid semantic versions {}", .0.join(", "))]
    NoSemanticVersionsFound(Vec<String>),
    #[error("failed to resolve {usage}: {reason}")]
    NoResolve {
        usage: ResolutionInfo,
        reason: String,
    },
    #[error("{usage} was not found: {reason}")]
    NotFound {
        usage: ResolutionInfo,
        reason: String,
    },
    #[error("{usage} is not supported: {reason}")]
    UnsupportedUsage {
        usage: ResolutionInfo,
        reason: String,
    },
    #[error("failure during resolution")]
    Resolution(#[from] Error),
}

#[expect(clippy::result_large_err)]
pub fn do_info<R: ResolveRead>(
    uri: &Iri<String>,
    resolver: &R,
) -> Result<(InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw), InfoError<R::Error>> {
    // TODO: support other usage types
    let resolve = ResolutionInfo::iri(uri.to_owned());
    let outcome = resolver.resolve_read(&resolve)?;

    match outcome {
        ResolutionOutcome::Resolved(resolved) => {
            let mut it = resolved.into_iter().peekable();
            assert!(it.peek().is_some());

            let mut best_version_info_meta: Option<(Version, _, _)> = None;
            let mut non_semantic_versions: Vec<String> = Vec::new();

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
                match do_info_project(&candidate_project) {
                    Ok((info, meta)) => {
                        best_version_info_meta =
                            match (Version::parse(&info.version), &best_version_info_meta) {
                                (Ok(cur_version), Some((best_version, _, _)))
                                    if &cur_version > best_version =>
                                {
                                    Some((cur_version, info, meta))
                                }
                                (Ok(_), Some(_)) => best_version_info_meta,
                                (Ok(cur_version), None) => Some((cur_version, info, meta)),
                                (Err(_), _) => {
                                    non_semantic_versions.push(info.version);
                                    best_version_info_meta
                                }
                            }
                    }
                    Err(err) => {
                        log::warn!("ignoring a project because: {}", format_err(err));
                    }
                };
            }
            match best_version_info_meta {
                Some((_, info, meta)) => {
                    if !non_semantic_versions.is_empty() {
                        log::warn!(
                            "the following versions were skipped as they are not semantic versions {}",
                            non_semantic_versions.join(", ")
                        );
                    }
                    Ok((info, meta))
                }
                None => Err(InfoError::NoSemanticVersionsFound(non_semantic_versions)),
            }
        }
        ResolutionOutcome::UnsupportedUsageType { reason } => Err(InfoError::UnsupportedUsage {
            usage: resolve,
            reason,
        }),
        ResolutionOutcome::Unresolvable { reason } => Err(InfoError::NoResolve {
            usage: resolve,
            reason,
        }),
        ResolutionOutcome::NotFound { reason } => Err(InfoError::NotFound {
            usage: resolve,
            reason,
        }),
    }
}
