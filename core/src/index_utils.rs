// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    env::iri_normalize::{IriNormalizeError, canonicalize_iri},
    purl::{PKG_SYSAND_PREFIX, SysandPurlError, parse_sysand_purl},
};

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
