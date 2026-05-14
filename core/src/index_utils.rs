// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    iri_normalize::{IriNormalizeError, canonicalize_iri},
    model::InterchangeProjectUsageRaw,
    purl::{PKG_SYSAND_PREFIX, SysandPurlError, parse_sysand_purl},
};

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
    #[serde(default)]
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
/// [`Status::Available`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
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
    /// deserializes as [`Status::Available`].
    #[serde(default)]
    pub(crate) status: Status,
}

impl Status {
    /// Predicate for `#[serde(skip_serializing_if = "...")]` so emitters
    /// drop `status` when it would round-trip as the default.
    #[allow(dead_code)]
    pub(crate) fn is_available(&self) -> bool {
        matches!(self, Status::Available)
    }
}

pub const IRI_HASH_SEGMENT: &str = "_iri";

pub fn hash_uri<S: AsRef<str>>(uri: S) -> String {
    let digest = Sha256::digest(uri.as_ref());
    format!("{:x}", digest)
}

#[derive(Debug, Clone)]
pub enum ParsedIri {
    Sysand { publisher: String, name: String },
    Other { normalized_iri: String },
}

impl ParsedIri {
    pub fn to_path_segments(self) -> Vec<String> {
        match self {
            ParsedIri::Sysand { publisher, name } => vec![publisher, name],
            ParsedIri::Other { normalized_iri } => {
                vec![IRI_HASH_SEGMENT.to_string(), hash_uri(normalized_iri)]
            }
        }
    }

    pub fn to_iri(self) -> String {
        match self {
            ParsedIri::Sysand { publisher, name } => {
                format!("{}{}/{}", PKG_SYSAND_PREFIX, publisher, name)
            }
            ParsedIri::Other { normalized_iri } => normalized_iri,
        }
    }
}

#[derive(Debug, Error)]
pub enum ParseIriError {
    #[error("cannot canonicalize IRI `{iri}` for `_iri` bucket")]
    MalformedIri {
        iri: Box<str>,
        #[source]
        source: IriNormalizeError,
    },
    #[error("malformed `pkg:sysand` IRI `{iri}`")]
    MalformedSysandPurl {
        iri: Box<str>,
        #[source]
        source: SysandPurlError,
    },
}

/// Parse an IRI to later construct the index path segments that locate its project directory.
/// The detailed wire mapping is specified in `docs/src/index-protocol.md`;
/// this function keeps malformed `pkg:sysand/...` IRIs out of the generic
/// `_iri/<hash>/` bucket so user typos fail loudly.
pub fn parse_iri(iri: &str) -> Result<ParsedIri, ParseIriError> {
    match parse_sysand_purl(iri) {
        Ok(Some((publisher, name))) => Ok(ParsedIri::Sysand {
            publisher: publisher.to_string(),
            name: name.to_string(),
        }),
        Ok(None) => {
            let malformed = |source| ParseIriError::MalformedIri {
                iri: iri.into(),
                source,
            };
            let parsed =
                fluent_uri::Iri::parse(iri).map_err(|e| malformed(IriNormalizeError::Parse(e)))?;
            let normalized_iri = canonicalize_iri(parsed).map_err(malformed)?;
            Ok(ParsedIri::Other { normalized_iri })
        }
        Err(source) => Err(ParseIriError::MalformedSysandPurl {
            iri: iri.into(),
            source,
        }),
    }
}
