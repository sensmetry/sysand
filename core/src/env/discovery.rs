// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Discovery of `index_root` and `api_root` via
//! `<discovery-root>/sysand-index-config.json`. The wire contract lives in
//! `design/index-protocol.md`; this module implements the client-side fetch
//! and URL-shape validation.
//!
//! The underlying `reqwest` middleware applies its default redirect policy
//! to the discovery fetch (see the comment next to
//! [`crate::resolve::net_utils::create_reqwest_client`]).

use serde::Deserialize;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    env::index::{HttpFetchError, IndexEnvironmentError, MissingPolicy, fetch_json},
    index::iri::parse_iri,
    index_location::{IndexLocation, IndexLocationError, with_trailing_slash},
};

const INDEX_PATH: &str = "index.json";
const VERSIONS_PATH: &str = "versions.json";
const KPAR_FILE: &str = "project.kpar";
const PROJECT_JSON_FILE: &str = ".project.json";
const META_JSON_FILE: &str = ".meta.json";

/// Resolved view of a sysand index server's two roots, as produced by the
/// discovery step.
#[derive(Debug, Clone)]
pub struct ResolvedEndpoints {
    /// Location of the sysand index (where `index.json` lives): a plain
    /// base URL or a `{path}` URL template.
    pub index_root: IndexLocation,
    /// Base URL of the sysand index API (where `v1/upload` lives). `None`
    /// unless the discovery document advertises `api_root`; a plain
    /// discovery root is not treated as an implicit API. An index that
    /// does not advertise `api_root` is read-only from this client's
    /// point of view.
    pub api_root: Option<url::Url>,
    /// Whether the discovery document explicitly supplied `api_root`, as
    /// opposed to the runtime default (a plain discovery root doubling as
    /// `api_root`, including the absent-document [`ResolvedEndpoints::flat`]
    /// case). When true, `api_root` is always `Some`. Consumed by
    /// credential validation (design/credential-storage.md section 5): only
    /// an advertised API surface is probed, so a static plain-URL index is
    /// never phantom-probed for an API it does not have.
    pub api_root_advertised: bool,
}

impl ResolvedEndpoints {
    /// Build a `ResolvedEndpoints` that routes index traffic at the
    /// discovery root itself. Used when the discovery document is absent
    /// (HTTP 404). Such an index advertises no `api_root`, so it exposes
    /// no API surface (it is read-only from this client's point of view).
    pub fn flat(discovery_root: IndexLocation) -> Self {
        Self {
            index_root: discovery_root,
            api_root: None,
            api_root_advertised: false,
        }
    }

    fn resolve<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> url::Url {
        self.index_root.resolve(segments)
    }

    pub(crate) fn index_url(&self) -> url::Url {
        self.resolve([INDEX_PATH])
    }

    /// The project directory's two path segments for `iri`
    /// (`[<publisher>, <name>]` or `[_iri, <sha256hex>]`). Returned as
    /// separate segments so each is encoded independently, avoiding a
    /// join-then-split round-trip.
    fn project_rel_segments<S: AsRef<str>>(iri: S) -> Result<[String; 2], IndexEnvironmentError> {
        Ok(parse_iri(iri.as_ref())?.get_path_segments())
    }

    pub(crate) fn kpar_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let project = Self::project_rel_segments(iri)?;
        Ok(self.resolve(
            project
                .iter()
                .map(String::as_str)
                .chain([version.as_ref(), KPAR_FILE]),
        ))
    }

    pub(crate) fn project_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let project = Self::project_rel_segments(iri)?;
        Ok(self.resolve(
            project
                .iter()
                .map(String::as_str)
                .chain([version.as_ref(), PROJECT_JSON_FILE]),
        ))
    }

    pub(crate) fn meta_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let project = Self::project_rel_segments(iri)?;
        Ok(self.resolve(
            project
                .iter()
                .map(String::as_str)
                .chain([version.as_ref(), META_JSON_FILE]),
        ))
    }

    pub(crate) fn versions_url<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let project = Self::project_rel_segments(iri)?;
        Ok(self.resolve(project.iter().map(String::as_str).chain([VERSIONS_PATH])))
    }
}

#[derive(Debug, Deserialize)]
struct IndexConfigRaw {
    #[serde(default)]
    index_root: Option<String>,
    #[serde(default)]
    api_root: Option<String>,
}

/// Errors that can occur during the discovery step. Surface as
/// [`crate::env::index::IndexEnvironmentError::Discovery`] at the env
/// boundary.
#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error(transparent)]
    Fetch(#[from] HttpFetchError),
    #[error(
        "discovery document at `{url}` supplied a relative URL `{value}` for `{field}`;\n\
         absolute HTTP(S) URLs are required"
    )]
    RelativeUrl {
        url: Box<str>,
        field: &'static str,
        value: String,
    },
    #[error(
        "discovery document at `{url}` supplied an invalid URL `{value}` for `{field}`:\n\
         {source}"
    )]
    InvalidUrl {
        url: Box<str>,
        field: &'static str,
        value: String,
        source: url::ParseError,
    },
    #[error(
        "discovery document at `{url}` supplied a non-HTTP(S) URL `{value}` for `{field}`;\n\
         only `http` and `https` are supported"
    )]
    UnsupportedScheme {
        url: Box<str>,
        field: &'static str,
        value: String,
    },
    #[error(
        "discovery document at `{url}` supplied URL userinfo in `{value}` for `{field}`;\n\
         username and password are not allowed"
    )]
    Userinfo {
        url: Box<str>,
        field: &'static str,
        value: String,
    },
    #[error("discovery document at `{url}` supplied an invalid `{field}`")]
    InvalidLocation {
        url: Box<str>,
        field: &'static str,
        source: IndexLocationError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpBaseUrlShapeError {
    UnsupportedScheme,
    Userinfo,
}

pub(crate) fn validate_http_base_url_shape(url: &url::Url) -> Result<(), HttpBaseUrlShapeError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(HttpBaseUrlShapeError::UnsupportedScheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(HttpBaseUrlShapeError::Userinfo);
    }
    Ok(())
}

fn discovery_shape_error(
    source_url: &url::Url,
    field: &'static str,
    value: &url::Url,
    error: HttpBaseUrlShapeError,
) -> DiscoveryError {
    match error {
        HttpBaseUrlShapeError::UnsupportedScheme => DiscoveryError::UnsupportedScheme {
            url: source_url.as_str().into(),
            field,
            value: value.as_str().to_owned(),
        },
        HttpBaseUrlShapeError::Userinfo => DiscoveryError::Userinfo {
            url: source_url.as_str().into(),
            field,
            value: value.as_str().to_owned(),
        },
    }
}

/// Fetch the discovery document from
/// `<discovery_root>/sysand-index-config.json` and produce the resolved
/// `(index_root, api_root)` pair. See module docs for the protocol-level
/// semantics.
pub async fn fetch_index_config<P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    discovery_root: &IndexLocation,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    // `IndexLocation` enforces its own invariants at construction — a
    // plain root is already an absolute HTTP(S) URL, without userinfo, and
    // with a trailing slash so relative-path resolution treats it as a
    // directory — so no normalization is needed here.
    let discovery_location = discovery_root.clone();

    let config_url = discovery_location.resolve(["sysand-index-config.json"]);

    let parsed: Option<IndexConfigRaw> =
        fetch_json(client, auth, &config_url, MissingPolicy::AllowNotFound).await?;

    let Some(raw) = parsed else {
        let endpoints = ResolvedEndpoints::flat(discovery_location);
        log_resolved(&endpoints);
        return Ok(endpoints);
    };

    // Parse a supplied field value as a plain base URL.
    // `url::Url::parse` on a relative input (e.g. `"/index/"`) returns
    // `Err(RelativeUrlWithoutBase)` — map that specifically to
    // `RelativeUrl` so the error is actionable.
    let parse_base_url = |field: &'static str, s: String| -> Result<url::Url, DiscoveryError> {
        let parsed = match url::Url::parse(&s) {
            Ok(parsed) => parsed,
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                return Err(DiscoveryError::RelativeUrl {
                    url: config_url.as_str().into(),
                    field,
                    value: s,
                });
            }
            Err(source) => {
                return Err(DiscoveryError::InvalidUrl {
                    url: config_url.as_str().into(),
                    field,
                    value: s,
                    source,
                });
            }
        };
        validate_http_base_url_shape(&parsed)
            .map_err(|error| discovery_shape_error(&config_url, field, &parsed, error))?;
        Ok(with_trailing_slash(parsed))
    };

    // `index_root` may itself be a URL template; `api_root` may not
    // (uploads are not file fetches, so templating it is meaningless).
    // Both roots enforce their validity invariants through
    // `IndexLocation::parse`, so a plain and a templated `index_root` are
    // validated in the same place.
    let index_root = match raw.index_root {
        None => discovery_location.clone(),
        Some(s) => IndexLocation::parse(&s).map_err(|source| DiscoveryError::InvalidLocation {
            url: config_url.as_str().into(),
            field: "index_root",
            source,
        })?,
    };

    // The API surface exists only when the discovery document advertises
    // `api_root`; a plain discovery root is not an implicit API (see the
    // `ResolvedEndpoints::api_root` field doc).
    let api_root_advertised = raw.api_root.is_some();
    let api_root = match raw.api_root {
        Some(s) => Some(parse_base_url("api_root", s)?),
        None => None,
    };

    let endpoints = ResolvedEndpoints {
        index_root,
        api_root,
        api_root_advertised,
    };
    log_resolved(&endpoints);
    Ok(endpoints)
}

/// Discovery decides where every later index fetch goes, so record the
/// outcome for debugging.
fn log_resolved(endpoints: &ResolvedEndpoints) {
    match &endpoints.api_root {
        Some(api_root) => log::debug!(
            "resolved index endpoints: index_root `{}`, api_root `{api_root}`",
            endpoints.index_root
        ),
        None => log::debug!(
            "resolved index endpoints: index_root `{}`, no api_root (read-only index)",
            endpoints.index_root
        ),
    }
}

#[cfg(test)]
#[path = "./discovery_tests.rs"]
mod tests;
