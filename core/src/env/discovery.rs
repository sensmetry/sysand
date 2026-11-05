// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Discovery of `index_root` and `api_root` via
//! `<discovery-root>/sysand-index-config.json`. The wire contract lives in
//! `design/index-protocol.md`; this module implements the client-side fetch
//! and URL-shape validation.
//!
//! The discovery fetch follows redirects under the `reqwest` default policy.

use serde::Deserialize;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    env::index::{HttpFetchError, IndexEnvironmentError, MissingPolicy, fetch_json},
    index::iri::parse_iri,
    index_location::{IndexLocation, IndexLocationError, with_trailing_slash},
    redact::RedactedUrl,
};

/// Path of the discovery document under the discovery root
pub(crate) const INDEX_CONFIG_PATH: &str = "sysand-index-config.json";

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
    /// unless the discovery document advertises `api_root`; without it the
    /// index is read-only.
    pub api_root: Option<url::Url>,
}

impl ResolvedEndpoints {
    /// Endpoints that route index traffic at the discovery root itself, used
    /// when the discovery document is absent (HTTP 404). There is no
    /// `api_root`, so the index is read-only.
    pub fn flat(discovery_root: IndexLocation) -> Self {
        Self {
            index_root: discovery_root,
            api_root: None,
        }
    }

    fn resolve<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> url::Url {
        self.index_root.resolve(segments)
    }

    pub(crate) fn index_url(&self) -> url::Url {
        self.resolve([INDEX_PATH])
    }

    /// The project directory's two path segments for `iri`
    /// (`[<publisher>, <name>]` or `[_iri, <sha256hex>]`), kept separate so
    /// each is encoded independently.
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

/// Wire shape of the discovery document.
#[derive(Debug, Deserialize)]
pub(crate) struct IndexConfigRaw {
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
        url: RedactedUrl,
        field: &'static str,
        value: String,
    },
    #[error("discovery document at `{url}` supplied an invalid URL `{value}` for `{field}`")]
    InvalidUrl {
        url: RedactedUrl,
        field: &'static str,
        value: String,
        source: url::ParseError,
    },
    #[error(
        "discovery document at `{url}` supplied a non-HTTP(S) URL `{value}` for `{field}`;\n\
         only `http` and `https` are supported"
    )]
    UnsupportedScheme {
        url: RedactedUrl,
        field: &'static str,
        value: String,
    },
    #[error(
        "discovery document at `{url}` supplied URL userinfo in `{value}` for `{field}`;\n\
         username and password are not allowed"
    )]
    Userinfo {
        url: RedactedUrl,
        field: &'static str,
        value: String,
    },
    #[error("discovery document at `{url}` supplied an invalid `{field}`")]
    InvalidLocation {
        url: RedactedUrl,
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
            url: source_url.clone().into(),
            field,
            value: value.as_str().to_owned(),
        },
        HttpBaseUrlShapeError::Userinfo => DiscoveryError::Userinfo {
            url: source_url.clone().into(),
            field,
            value: value.as_str().to_owned(),
        },
    }
}

/// Fetch the discovery document from
/// `<discovery_root>/sysand-index-config.json` and produce the resolved
/// `(index_root, api_root)` pair.
pub async fn fetch_index_config<P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    discovery_root: &IndexLocation,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    fetch_index_config_with(client, auth, discovery_root, MissingPolicy::AllowNotFound).await
}

/// [`fetch_index_config`], except an absent document (HTTP 404) surfaces
/// as [`HttpFetchError::BadHttpStatus`] and does not fold into the flat
/// topology
pub(crate) async fn fetch_index_config_strict<P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    discovery_root: &IndexLocation,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    fetch_index_config_with(client, auth, discovery_root, MissingPolicy::RequirePresent).await
}

async fn fetch_index_config_with<P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    discovery_root: &IndexLocation,
    missing: MissingPolicy,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    let discovery_location = discovery_root.clone();

    let config_url = discovery_location.resolve([INDEX_CONFIG_PATH]);

    let parsed: Option<IndexConfigRaw> = fetch_json(client, auth, &config_url, missing).await?;

    let Some(raw) = parsed else {
        let endpoints = ResolvedEndpoints::flat(discovery_location);
        log_resolved(&endpoints);
        return Ok(endpoints);
    };

    resolve_index_config(raw, &config_url, discovery_location)
}

/// Interpret a parsed discovery document fetched from `config_url` into
/// the resolved endpoints, validating the URL shapes it supplies.
pub(crate) fn resolve_index_config(
    raw: IndexConfigRaw,
    config_url: &url::Url,
    discovery_location: IndexLocation,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    // Parse a supplied field value as a plain base URL. A relative input
    // (e.g. `"/index/"`) gets its own `RelativeUrl` error.
    let parse_base_url = |field: &'static str, s: String| -> Result<url::Url, DiscoveryError> {
        let parsed = match url::Url::parse(&s) {
            Ok(parsed) => parsed,
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                return Err(DiscoveryError::RelativeUrl {
                    url: config_url.clone().into(),
                    field,
                    value: s,
                });
            }
            Err(source) => {
                return Err(DiscoveryError::InvalidUrl {
                    url: config_url.clone().into(),
                    field,
                    value: s,
                    source,
                });
            }
        };
        validate_http_base_url_shape(&parsed)
            .map_err(|error| discovery_shape_error(config_url, field, &parsed, error))?;
        Ok(with_trailing_slash(parsed))
    };

    // `index_root` may be a URL template; `api_root` may not (uploads are
    // not file fetches).
    let index_root = match raw.index_root {
        None => discovery_location,
        Some(s) => IndexLocation::parse(&s).map_err(|source| DiscoveryError::InvalidLocation {
            url: config_url.clone().into(),
            field: "index_root",
            source,
        })?,
    };

    // A plain discovery root is not an implicit API.
    let api_root = match raw.api_root {
        Some(s) => Some(parse_base_url("api_root", s)?),
        None => None,
    };

    let endpoints = ResolvedEndpoints {
        index_root,
        api_root,
    };
    log_resolved(&endpoints);
    Ok(endpoints)
}

/// Log the resolved endpoints at debug level.
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
