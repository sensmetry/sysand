// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! List every version a resolver can offer for a project.

use fluent_uri::Iri;
use semver::Version;

use crate::{
    info::InfoError,
    project::ProjectRead as _,
    resolve::{ResolutionInfo, ResolutionOutcome, ResolveRead},
    utils::format_err,
};

/// The versions a resolver offers for one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionListing {
    /// The IRI that was looked up.
    pub iri: String,
    /// Distinct semver-valid versions, highest first. Over an index these
    /// are the *available* versions: yanked and removed entries are never
    /// enumerated.
    pub versions: Vec<Version>,
    /// Version strings that are not valid semver, in encounter order. Kept
    /// for diagnostics rather than dropped silently; an index cannot publish
    /// such entries, but other sources can.
    pub ignored: Vec<String>,
}

/// List the versions of `iri` known to `resolver`.
///
/// The sibling of [`crate::info::do_info`]: the same resolution, but every
/// candidate's version is collected instead of keeping the best one. Over an
/// index this costs the per-project `versions.json` request only — each
/// candidate answers `version()` from the advertised entry.
///
/// Candidates that fail to load, or report no version, are skipped with a
/// log line, as `do_info` skips them. If every candidate was skipped the
/// result is [`InfoError::NoSemanticVersionsFound`] with an empty list; if
/// only non-semver versions were found the listing is `Ok` with an empty
/// `versions` and a full `ignored`, so the caller can report them.
#[expect(clippy::result_large_err)]
pub fn do_versions<R: ResolveRead>(
    iri: &Iri<String>,
    resolver: &R,
) -> Result<VersionListing, InfoError<R::Error>> {
    let resolve = ResolutionInfo::iri(iri.to_owned());
    let outcome = resolver.resolve_read(&resolve)?;

    let resolved = match outcome {
        ResolutionOutcome::Resolved(resolved) => resolved,
        ResolutionOutcome::UnsupportedUsageType { reason } => {
            return Err(InfoError::UnsupportedUsage {
                usage: resolve,
                reason,
            });
        }
        ResolutionOutcome::NotFound { reason } => {
            return Err(InfoError::NotFound {
                usage: resolve,
                reason,
            });
        }
        ResolutionOutcome::Unresolvable { reason } => {
            return Err(InfoError::NoResolve {
                usage: resolve,
                reason,
            });
        }
    };

    let mut versions: Vec<Version> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    let mut seen_any = false;

    for candidate in resolved {
        let project = match candidate {
            Ok(project) => project,
            Err(e) => {
                // As in `do_info`: the candidate list is every possibility,
                // and some are expected not to work.
                log::debug!("skipping candidate project: {}", format_err(e));
                continue;
            }
        };
        match project.version() {
            Ok(Some(version)) => {
                seen_any = true;
                match Version::parse(&version) {
                    Ok(parsed) => versions.push(parsed),
                    Err(_) => ignored.push(version),
                }
            }
            Ok(None) => {
                log::warn!("ignoring a candidate of `{iri}` that declares no version");
            }
            Err(err) => {
                log::warn!("ignoring a project because: {}", format_err(err));
            }
        }
    }

    if !seen_any {
        return Err(InfoError::NoSemanticVersionsFound(Vec::new()));
    }

    // Reverse sort
    versions.sort_unstable_by(|a, b| b.cmp(a));
    versions.dedup();

    Ok(VersionListing {
        iri: iri.to_string(),
        versions,
        ignored,
    })
}

#[cfg(test)]
#[path = "./versions_tests.rs"]
mod tests;
