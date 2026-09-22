// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

#![expect(clippy::field_scoped_visibility_modifiers)]

use std::num::NonZeroU64;

use semver::Version;
use thiserror::Error;

use serde::{Deserialize, Serialize};

use crate::model::InterchangeProjectUsageRaw;

/// Top-level `index.json` — the list of every project IRI the index knows
/// about. Used by `uris_async` for list-all enumeration. Per-project version
/// data lives in `versions.json`.
#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct IndexJson {
    pub(crate) projects: Vec<IndexProject>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct IndexProject {
    pub(crate) iri: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub(crate) status: ProjectStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectStatus {
    #[default]
    Available,
    Removed,
}

/// Retirement state of a `versions.json` entry; see the index protocol for
/// the wire contract and transition rules. An omitted `status` parses as
/// [`VersionStatus::Available`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VersionStatus {
    #[default]
    Available,
    Yanked,
    Removed,
}

/// Per-project `versions.json`: enough to enumerate candidates and
/// verify archives without downloading first. The publish-time artifact
/// metadata (`kpar_size`, `kpar_digest`) lets the
/// client populate the lockfile lazily; `.project.json` / `.meta.json`
/// are only fetched once a specific version is materialized, and the
/// client reconciles them against these digests before exposing either.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct VersionsJson {
    pub(crate) versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VersionEntry {
    pub(crate) version: String,
    /// Required so the solver can run on `versions.json` alone, without
    /// fetching each candidate's `.project.json`.
    pub(crate) usage: Vec<InterchangeProjectUsageRaw>,
    /// Byte length of the kpar archive; lets `sources_async` skip a HEAD.
    pub(crate) kpar_size: NonZeroU64,
    /// Digest of the kpar archive bytes, verified against the streamed
    /// body when the archive is downloaded.
    pub(crate) kpar_digest: String,
    /// Retirement state (§8). Optional on the wire; an omitted field
    /// deserializes as [`VersionStatus::Available`].
    #[serde(default, skip_serializing_if = "is_default")]
    pub(crate) status: VersionStatus,
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}

/// A validated sha256 hex digest — 64 lowercase hex characters, with the
/// `"sha256:"` prefix already stripped. Constructed via `TryFrom<&str>`,
/// which validates a `versions.json` digest once: a value of this type
/// needs no second "is this hex?" check at the point of use. Uppercase
/// hex is rejected rather than normalized —
/// the wire format is lowercase-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sha256HexDigest(String);

impl Sha256HexDigest {
    pub(crate) fn as_hex(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Sha256HexDigest {
    type Error = ();

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        let hex = raw.strip_prefix("sha256:").ok_or(())?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|c| c.is_ascii_digit() || matches!(c, b'a'..=b'f'))
        {
            return Err(());
        }
        Ok(Self(hex.to_owned()))
    }
}

/// A `versions.json` entry after ingest-time validation: `version` is parsed
/// into `semver::Version`, digests are validated lowercase hex without the
/// wire prefix, and `usage`/non-zero `kpar_size` are carried through. This
/// is the representation the rest of the crate sees — the raw
/// [`VersionEntry`] is only alive briefly during deserialization.
#[derive(Debug, Clone)]
pub(crate) struct AdvertisedVersion {
    pub(crate) version: Version,
    pub(crate) usage: Vec<InterchangeProjectUsageRaw>,
    pub(crate) kpar_size: NonZeroU64,
    pub(crate) kpar_digest: Sha256HexDigest,
    pub(crate) status: VersionStatus,
}

/// A `versions.json` document that breaks the shape rules of
/// `design/index-protocol.md` §8. `location` is the URL or file the
/// document was read from.
#[derive(Error, Debug)]
pub enum VersionsJsonError {
    #[error(
        "versions.json at `{location}` has entry for version `{version}` with \
         malformed `{field}` = `{value}` (expected `sha256:<64-hex>`)"
    )]
    InvalidVersionEntry {
        location: Box<str>,
        version: String,
        field: &'static str,
        value: String,
    },
    #[error("versions.json at `{location}` has entry with non-semver version `{value}`: {source}")]
    InvalidSemverVersion {
        location: Box<str>,
        value: String,
        #[source]
        source: semver::Error,
    },
    #[error(
        "versions.json at `{location}` has entry with version `{value}` carrying build metadata; \
         build metadata is not permitted in index-advertised versions"
    )]
    VersionHasBuildMetadata { location: Box<str>, value: String },
    #[error(
        "versions.json at `{location}` is not in descending semver precedence: \
         `{prev}` precedes `{curr}` in wire order"
    )]
    VersionsOutOfOrder {
        location: Box<str>,
        prev: String,
        curr: String,
    },
    #[error("versions.json at `{location}` lists version `{version}` more than once")]
    DuplicateVersion { location: Box<str>, version: String },
}

/// Validate every entry's required fields (semver parse, no build
/// metadata, digest shape, uniqueness) and emit the
/// `AdvertisedVersion`s the rest of the crate consumes. The check
/// runs at ingest rather than per-materialization: a cached document
/// is shared across candidate enumeration and lockfile assembly, so
/// catching a protocol violation up front avoids a delayed surprise
/// downstream.
///
/// Ordering is checked on the parsed `semver::Version`, not lexically
/// — violations surface as [`VersionsJsonError::VersionsOutOfOrder`],
/// which catches `10.0.0-beta.1` before `10.0.0`.
pub(crate) fn validate_versions(
    location: &str,
    vs: VersionsJson,
) -> Result<Vec<AdvertisedVersion>, VersionsJsonError> {
    let validated: Vec<AdvertisedVersion> = vs
        .versions
        .into_iter()
        .map(|entry| {
            let version = Version::parse(&entry.version).map_err(|source| {
                VersionsJsonError::InvalidSemverVersion {
                    location: location.into(),
                    value: entry.version.clone(),
                    source,
                }
            })?;
            // `semver::Version::parse` is lenient on the `+build…`
            // suffix; the wire-format rejection lives here.
            if !version.build.is_empty() {
                return Err(VersionsJsonError::VersionHasBuildMetadata {
                    location: location.into(),
                    value: entry.version,
                });
            }
            let kpar_digest =
                Sha256HexDigest::try_from(entry.kpar_digest.as_str()).map_err(|()| {
                    VersionsJsonError::InvalidVersionEntry {
                        location: location.into(),
                        version: entry.version.clone(),
                        field: "kpar_digest",
                        value: entry.kpar_digest.clone(),
                    }
                })?;
            Ok(AdvertisedVersion {
                version,
                usage: entry.usage,
                kpar_size: entry.kpar_size,
                kpar_digest,
                status: entry.status,
            })
        })
        .collect::<Result<_, _>>()?;
    // "Pick the best duplicate" has no principled answer — two entries
    // with the same semver may carry different digests. Letting them
    // reach the solver would leak non-determinism into pubgrub, so
    // reject here.
    let mut seen = std::collections::HashSet::new();
    for v in &validated {
        if !seen.insert(v.version.clone()) {
            return Err(VersionsJsonError::DuplicateVersion {
                location: location.into(),
                version: v.version.to_string(),
            });
        }
    }
    for pair in validated.windows(2) {
        if let [prev, curr] = pair
            && prev.version <= curr.version
        {
            return Err(VersionsJsonError::VersionsOutOfOrder {
                location: location.into(),
                prev: prev.version.to_string(),
                curr: curr.version.to_string(),
            });
        }
    }
    Ok(validated)
}
