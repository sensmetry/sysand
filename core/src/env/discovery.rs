// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Discovery of `index_root` and `api_root` via
//! `<discovery-root>/sysand-index-config.json`. The wire contract lives in
//! `docs/src/index-protocol.md`; this module implements the client-side fetch
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
    index_utils::parse_iri,
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
    /// Base URL of the sysand index (where `index.json` lives).
    pub index_root: url::Url,
    /// Base URL of the sysand index API (where `v1/upload` lives).
    pub api_root: url::Url,
}

impl ResolvedEndpoints {
    /// Build a `ResolvedEndpoints` that routes both index and API traffic
    /// at the discovery root itself. Used when the discovery document is
    /// absent (HTTP 404) or present with neither field set.
    pub fn flat(discovery_root: url::Url) -> Self {
        Self {
            index_root: discovery_root.clone(),
            api_root: discovery_root,
        }
    }

    fn url_join(url: &url::Url, join: &str) -> Result<url::Url, IndexEnvironmentError> {
        url.join(join)
            .map_err(|e| IndexEnvironmentError::JoinURL(url.as_str().into(), join.into(), e))
    }

    pub(crate) fn index_url(&self) -> Result<url::Url, IndexEnvironmentError> {
        Self::url_join(&self.index_root, INDEX_PATH)
    }

    pub(crate) fn project_url<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let parsed_iri = parse_iri(iri.as_ref())?;
        let path = parsed_iri.get_path();
        Self::url_join(&self.index_root, &format!("{path}/"))
    }

    /// Per-version directory URL ending with a trailing slash, so that
    /// `Url::join` treats it as a directory when composing leaf URLs
    /// (`project.kpar`, `.project.json`, `.meta.json`).
    pub(crate) fn version_dir_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let base = self.project_url(iri)?;
        Self::url_join(&base, &format!("{}/", version.as_ref()))
    }

    pub(crate) fn kpar_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, KPAR_FILE)
    }

    pub(crate) fn project_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, PROJECT_JSON_FILE)
    }

    pub(crate) fn meta_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, IndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, META_JSON_FILE)
    }

    pub(crate) fn versions_url<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<url::Url, IndexEnvironmentError> {
        let base = self.project_url(iri)?;
        Self::url_join(&base, VERSIONS_PATH)
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
        "discovery document at `{url}` supplied a relative URL `{value}` for `{field}`; \
         absolute HTTP(S) URLs are required"
    )]
    RelativeUrl {
        url: Box<str>,
        field: &'static str,
        value: String,
    },
    #[error(
        "discovery document at `{url}` supplied an invalid URL `{value}` for `{field}`: {source}"
    )]
    InvalidUrl {
        url: Box<str>,
        field: &'static str,
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error(
        "discovery document at `{url}` supplied a non-HTTP(S) URL `{value}` for `{field}`; \
         only `http` and `https` are supported"
    )]
    UnsupportedScheme {
        url: Box<str>,
        field: &'static str,
        value: String,
    },
    #[error(
        "discovery document at `{url}` supplied URL userinfo in `{value}` for `{field}`; \
         username and password are not allowed"
    )]
    Userinfo {
        url: Box<str>,
        field: &'static str,
        value: String,
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
    discovery_root: &url::Url,
) -> Result<ResolvedEndpoints, DiscoveryError> {
    validate_http_base_url_shape(discovery_root).map_err(|error| {
        discovery_shape_error(discovery_root, "<discovery_root>", discovery_root, error)
    })?;

    // Normalize the discovery root so `join` treats it as a directory.
    let directory_root = with_trailing_slash(discovery_root.clone());
    // Build the URL through `join` so that trailing slashes on the
    // discovery root behave consistently (RFC 3986 §5.3 path resolution).
    let config_url = directory_root
        .join("sysand-index-config.json")
        .expect("joining a fixed relative path onto an HTTP(S) base URL succeeds");

    let parsed: Option<IndexConfigRaw> =
        fetch_json(client, auth, &config_url, MissingPolicy::AllowNotFound).await?;

    let Some(raw) = parsed else {
        return Ok(ResolvedEndpoints::flat(directory_root));
    };

    let parse_field = |field: &'static str, value: Option<String>, default: &url::Url| {
        let Some(s) = value else {
            return Ok(default.clone());
        };
        let parsed = match url::Url::parse(&s) {
            Ok(parsed) => parsed,
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

    // `url::Url::parse` on a relative input (e.g. `"/index/"`) returns
    // `Err(RelativeUrlWithoutBase)` — map that specifically to
    // `RelativeUrl` so the error is actionable.
    let parse_or_relative =
        |field: &'static str, value: Option<String>, default: &url::Url| match parse_field(
            field, value, default,
        ) {
            Err(DiscoveryError::InvalidUrl {
                url,
                field,
                value,
                source: url::ParseError::RelativeUrlWithoutBase,
            }) => Err(DiscoveryError::RelativeUrl { url, field, value }),
            other => other,
        };

    let index_root = parse_or_relative("index_root", raw.index_root, &directory_root)?;
    let api_root = parse_or_relative("api_root", raw.api_root, &directory_root)?;

    Ok(ResolvedEndpoints {
        index_root,
        api_root,
    })
}

/// Return `url` with a guaranteed trailing slash on its path so that
/// `Url::join` treats it as a directory. Operates via `path_segments_mut`
/// rather than touching the serialized path string, so percent-encoded
/// segments survive the round-trip unchanged.
///
/// Callers must pass an HTTP(S) URL. Such URLs can be a base, so the
/// `path_segments_mut` call should not fail after endpoint-shape
/// validation.
pub(crate) fn with_trailing_slash(mut url: url::Url) -> url::Url {
    {
        let mut segments = url
            .path_segments_mut()
            .expect("caller passes a URL that can be a base");
        segments.pop_if_empty();
        segments.push("");
    }
    url
}

#[cfg(test)]
#[path = "./discovery_tests.rs"]
mod tests;
