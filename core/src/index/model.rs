// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::num::NonZeroU64;

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
/// metadata (`project_digest`, `kpar_size`, `kpar_digest`) lets the
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
    /// Canonical project digest (sha256 over canonicalized info+meta),
    /// used to populate the lockfile checksum without downloading the kpar.
    pub(crate) project_digest: String,
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

impl VersionStatus {
    /// Predicate for `#[serde(skip_serializing_if = "...")]` so emitters
    /// drop `status` when it would round-trip as the default.
    #[allow(dead_code)]
    pub(crate) fn is_available(&self) -> bool {
        matches!(self, VersionStatus::Available)
    }
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == T::default()
}
