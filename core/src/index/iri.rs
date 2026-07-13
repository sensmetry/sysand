// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    iri_normalize::{IriNormalizeError, canonicalize_iri},
    purl::{PKG_SYSAND_PREFIX, SysandPurlError, parse_sysand_purl},
    utils::sha256_lowercase_hex,
};

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
/// The detailed wire mapping is specified in `design/index-protocol.md`;
/// this function keeps malformed `pkg:sysand/...` IRIs out of the generic
/// `_iri/<hash>/` bucket so user typos fail loudly.
pub(crate) fn parse_iri(iri: &str) -> Result<ParsedIri, ParseIriError> {
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

pub(crate) const IRI_HASH_SEGMENT: &str = "_iri";

#[derive(Debug)]
pub(crate) enum ParsedIri {
    Sysand { publisher: String, name: String },
    Other { normalized_iri: String },
}

impl ParsedIri {
    /// The two path segments identifying the project directory:
    /// `[publisher, name]` for a sysand PURL, or `[_iri, <sha256hex>]` for
    /// any other IRI. Kept as separate segments (rather than a joined
    /// path) so callers can encode each independently.
    pub(crate) fn get_path_segments(&self) -> [String; 2] {
        match self {
            ParsedIri::Sysand { publisher, name } => [publisher.clone(), name.clone()],
            ParsedIri::Other { normalized_iri } => [
                IRI_HASH_SEGMENT.to_owned(),
                sha256_lowercase_hex(normalized_iri),
            ],
        }
    }

    pub(crate) fn get_path(&self) -> String {
        self.get_path_segments().join("/")
    }

    pub(crate) fn get_iri(&self) -> String {
        match self {
            ParsedIri::Sysand { publisher, name } => {
                format!("{}{}/{}", PKG_SYSAND_PREFIX, publisher, name)
            }
            ParsedIri::Other { normalized_iri } => normalized_iri.clone(),
        }
    }
}
