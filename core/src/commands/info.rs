// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use fluent_uri::Iri;
use semver::Version;
use thiserror::Error;

use crate::{
    env::utils::ErrorBound,
    project::{ProjectDefinition, ProjectRead},
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::format_err,
};

#[derive(Error, Debug)]
pub enum InfoProjectError<Error: ErrorBound> {
    // TODO: Add URI hints to projects and use them here
    #[error("project is missing")]
    MissingProject,
    #[error(transparent)]
    InvalidProject(#[from] Error),
}

pub fn do_info_project<P: ProjectRead>(
    project: &P,
) -> Result<ProjectDefinition, InfoProjectError<P::Error>> {
    project
        .get_project()
        .map_err(InfoProjectError::InvalidProject)?
        .ok_or(InfoProjectError::MissingProject)
}

#[derive(Error, Debug)]
pub enum InfoError<Error: ErrorBound> {
    #[error("none of the following found versions are valid semantic versions {}", .0.join(", "))]
    NoSemanticVersionsFound(Vec<String>),
    /// Every project the usage resolved to was skipped; `reasons` says why,
    /// one line per project.
    #[error(
        "{usage} resolved to no project that could be read:\n{}",
        reason_lines(reasons)
    )]
    NoReadableProject {
        usage: ResolutionInfo,
        reasons: Vec<String>,
    },
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

/// Same as [`do_info_usage`], for the project identified by `uri`
#[expect(clippy::result_large_err)]
pub fn do_info<R: ResolveRead>(
    uri: &Iri<String>,
    resolver: &R,
) -> Result<ProjectDefinition, InfoError<R::Error>> {
    do_info_usage(ResolutionInfo::iri(uri.to_owned()), resolver).map(|(definition, _)| definition)
}

/// Resolves `resolve` and returns the manifest of the highest (semantic)
/// version found, and that project. The version constraint of the usage,
/// if any, is not applied
#[expect(clippy::result_large_err)]
pub fn do_info_usage<R: ResolveRead>(
    resolve: ResolutionInfo,
    resolver: &R,
) -> Result<(ProjectDefinition, R::ProjectStorage), InfoError<R::Error>> {
    let outcome = resolver.resolve_read(&resolve)?;

    match outcome {
        ResolutionOutcome::Resolved(resolved) => {
            let mut it = resolved.into_iter().peekable();
            assert!(it.peek().is_some());

            let mut best: Option<(Version, ProjectDefinition, R::ProjectStorage)> = None;
            let mut non_semantic_versions: Vec<String> = Vec::new();
            let mut skipped = Skipped::default();

            for alt in it {
                let candidate_project = match alt {
                    Ok(cp) => cp,
                    Err(e) => {
                        skipped.unresolved.push(format_err(e));
                        continue;
                    }
                };
                match do_info_project(&candidate_project) {
                    Ok(definition) => {
                        let version = definition.version();
                        best = match (Version::parse(&version), &best) {
                            (Ok(cur_version), Some((best_version, _, _)))
                                if &cur_version > best_version =>
                            {
                                Some((cur_version, definition, candidate_project))
                            }
                            (Ok(_), Some(_)) => best,
                            (Ok(cur_version), None) => {
                                Some((cur_version, definition, candidate_project))
                            }
                            (Err(_), _) => {
                                non_semantic_versions.push(version);
                                best
                            }
                        }
                    }
                    Err(err) => skipped.unreadable.push(format_err(err)),
                }
            }
            match best {
                Some((_, definition, project)) => {
                    skipped.log();
                    if !non_semantic_versions.is_empty() {
                        log::warn!(
                            "the following versions were skipped as they are not semantic versions {}",
                            non_semantic_versions.join(", ")
                        );
                    }
                    Ok((definition, project))
                }
                None if non_semantic_versions.is_empty() => Err(skipped.into_error(resolve)),
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

/// Why the projects a usage resolved to were skipped.
#[derive(Debug, Default)]
pub(crate) struct Skipped {
    /// A project that was found but could not be read.
    pub unreadable: Vec<String>,
    /// A candidate the resolver could not open. The candidates are every
    /// possibility, and some are expected not to work.
    pub unresolved: Vec<String>,
}

impl Skipped {
    /// Logs every skip, for a resolution that found a project anyway.
    pub fn log(&self) {
        for reason in &self.unreadable {
            log::warn!("ignoring a project because: {reason}");
        }
        for reason in &self.unresolved {
            log::debug!("skipping candidate project: {reason}");
        }
    }

    /// The error for a resolution that found nothing usable: the
    /// unreadable projects, or the candidates when no project was found.
    pub fn into_error<E: ErrorBound>(self, usage: ResolutionInfo) -> InfoError<E> {
        let reasons = if self.unreadable.is_empty() {
            self.unresolved
        } else {
            self.unreadable
        };
        InfoError::NoReadableProject { usage, reasons }
    }
}

/// One reason per line, each continuation-aligned.
fn reason_lines(reasons: &[String]) -> String {
    reasons
        .iter()
        .map(|reason| format!("{SP:>8}{reason}", SP = ' '))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "./info_tests.rs"]
mod tests;
