// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP client for the sysand index protocol.
//!
//! This environment reads `index.json` for IRI enumeration and
//! per-project `versions.json` for candidate enumeration. A `versions.json`
//! entry carries the data needed to decide which version to lock plus the
//! artifact metadata needed for lockfile population and archive verification.
//!
//! Once a concrete version has been selected, the returned project wrapper
//! lazily fetches that version's real `.project.json` and `.meta.json` to
//! materialize the locked project record, while continuing to defer the kpar
//! download itself until archive contents are actually needed.

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

use semver::Version;
use serde::Deserialize;
use sha2::Sha256;
use thiserror::Error;
use tokio::sync::OnceCell;

use crate::{
    auth::{HTTPAuthentication, StandardHTTPAuthentication},
    env::{AsSyncEnvironmentTokio, ReadEnvironmentAsync, segment_uri_generic},
    model::InterchangeProjectUsageRaw,
    project::indexed_remote::{IndexedRemoteProject, IndexedRemoteProjectError},
    resolve::net_utils::json_get_request,
};

pub type HTTPIndexEnvironment =
    AsSyncEnvironmentTokio<HTTPIndexEnvironmentAsync<StandardHTTPAuthentication>>;

#[derive(Debug)]
pub struct HTTPIndexEnvironmentAsync<Policy> {
    pub client: reqwest_middleware::ClientWithMiddleware,
    pub auth_policy: Arc<Policy>,
    pub base_url: reqwest::Url,
    /// Intra-run cache of parsed `versions.json` documents, keyed by IRI.
    /// Avoids the duplicate fetches that otherwise occur because
    /// `versions_async` and `get_project_async` each independently hit the
    /// endpoint (and `get_project_async` is called once per candidate during
    /// solving). The cache is scoped to one env lifetime and never invalidates
    /// — there is no freshness signal in the protocol yet, and a single
    /// `sysand lock` run should see a stable view of the index.
    ///
    /// Each entry is a per-IRI `OnceCell`, so concurrent callers requesting
    /// the same IRI share a single fetch even under a parallel solver. A
    /// cached `None` (project missing) is stored just like a cached hit, so
    /// 404s are not re-queried within one env lifetime either.
    ///
    /// Cached `VersionsJson` documents are **validated and sorted
    /// newest-first** by parsed `semver::Version` (see
    /// `fetch_versions_json`).
    pub(crate) versions_cache: Mutex<HashMap<String, VersionsCacheEntry>>,
}

/// Per-IRI cache slot: a `OnceCell` shared by all concurrent callers
/// requesting the same IRI, holding either the parsed `versions.json`
/// (`Some`) or a "project missing" marker (`None`). Lifted to a type alias
/// to keep `versions_cache`'s signature readable.
pub(crate) type VersionsCacheEntry = Arc<OnceCell<Option<Arc<VersionsJson>>>>;

#[derive(Error, Debug)]
pub enum HTTPIndexEnvironmentError {
    #[error("failed to extend URL `{0}` with path `{1}`: {2}")]
    JoinURL(Box<str>, String, url::ParseError),
    #[error("error making an HTTP request:\n{0:#?}")]
    HTTPRequest(#[from] reqwest_middleware::Error),
    #[error("HTTP request to `{url}` returned status {status}")]
    BadHttpStatus {
        url: Box<str>,
        status: reqwest::StatusCode,
    },
    #[error("failed to read HTTP response: {0}")]
    HttpIo(io::Error),
    #[error("failed to parse JSON from `{url}`: {source}")]
    JsonParse {
        url: Box<str>,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "versions.json at `{url}` has entry for version `{version}` with \
         malformed `{field}` = `{value}` (expected `sha256:<64-hex>`)"
    )]
    InvalidVersionEntry {
        url: Box<str>,
        version: String,
        field: &'static str,
        value: String,
    },
    #[error("versions.json at `{url}` has entry with non-semver version `{value}`: {source}")]
    InvalidSemverVersion {
        url: Box<str>,
        value: String,
        #[source]
        source: semver::Error,
    },
    #[error("versions.json at `{url}` does not list version `{version}`")]
    VersionNotInIndex { url: Box<str>, version: String },
    #[error(transparent)]
    Project(#[from] Box<IndexedRemoteProjectError>),
}

const INDEX_PATH: &str = "index.json";
const VERSIONS_PATH: &str = "versions.json";
const KPAR_FILE: &str = "project.kpar";
const PROJECT_JSON_FILE: &str = ".project.json";
const META_JSON_FILE: &str = ".meta.json";
const IRI_HASH_SEGMENT: &str = "_iri";
const PKG_SYSAND_PREFIX: &str = "pkg:sysand/";

// Note on forward compatibility: none of the types below set
// `#[serde(deny_unknown_fields)]`. Unknown fields are silently ignored so
// servers can add new optional fields without breaking clients.
//
// The protocol currently has no schema-version signal. When a mechanism is
// chosen (URL prefix, media type, or a single in-document field) it should
// be added in one place — not duplicated across documents.

/// Top-level `index.json` — the list of every project IRI the index knows
/// about. Used by `uris_async` for list-all enumeration. Per-project version
/// data lives in `versions.json`.
#[derive(Debug, Deserialize)]
struct IndexJson {
    projects: Vec<IndexProject>,
}

#[derive(Debug, Deserialize)]
struct IndexProject {
    iri: String,
}

/// Per-project `versions.json`. Each entry carries the data needed to decide
/// which version to lock (`version` and `usage`) plus the publish-time
/// artifact metadata (`project_digest`, `kpar_size`, `kpar_digest`) that lets
/// the client populate the lockfile and verify archive integrity without
/// downloading first.
///
/// Protocol contract: `versions.json` is sufficient for candidate
/// enumeration/version selection. Once a specific version has been chosen,
/// the client may fetch that version's `.project.json` and `.meta.json` to
/// materialize the locked project record and reconcile it against the
/// digests advertised here. All five per-entry fields are required by the
/// protocol — a missing field on any entry makes the whole document reject
/// as malformed.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionsJson {
    versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct VersionEntry {
    version: String,
    // `usage` is a required protocol field because `versions.json` is
    // sufficient for candidate enumeration/version selection, so the solver
    // must be able to consume it without fetching `.project.json`.
    usage: Vec<InterchangeProjectUsageRaw>,
    /// Canonical project digest (sha256 over canonicalized info+meta),
    /// used to populate the lockfile checksum without downloading the kpar.
    /// Format: `"sha256:<lowercase-hex>"`.
    project_digest: String,
    /// Byte length of the kpar archive, used by `sources_async` in lieu of
    /// a HEAD request.
    kpar_size: u64,
    /// Digest of the kpar archive bytes, verified against the streamed body
    /// when the archive is downloaded. Format: `"sha256:<lowercase-hex>"`.
    kpar_digest: String,
}

/// Parse a wire-format `"sha256:<hex>"` digest into the lockfile's bare
/// lowercase-hex representation. Returns `None` if the algorithm prefix is
/// missing, the algorithm isn't `sha256`, or the hex body isn't exactly 64
/// hexadecimal characters — callers surface a protocol-level error in that
/// case since all digest fields in `versions.json` are required.
fn parse_sha256_digest(raw: &str) -> Option<String> {
    let hex = raw.strip_prefix("sha256:")?;
    if hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

/// Parse `pkg:sysand/<publisher>/<name>` into `(publisher, name)` when the IRI
/// matches exactly that shape (two slash-separated components after the
/// scheme) and both segments are valid, normalized `pkg:sysand` identifiers
/// per [`crate::purl`]. An IRI that fails any of these checks falls through
/// to the `_iri/<sha256>/` route, so a malicious or non-canonical IRI in a
/// dependency tree cannot traverse out of the configured index base.
fn parse_pkg_sysand_iri(iri: &str) -> Option<(&str, &str)> {
    use crate::purl::is_normalized_field;

    let rest = iri.strip_prefix(PKG_SYSAND_PREFIX)?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        // publisher: no dots; name: dots allowed
        [publisher, name]
            if is_normalized_field(publisher, false) && is_normalized_field(name, true) =>
        {
            Some((publisher, name))
        }
        _ => None,
    }
}

/// Map an IRI to the index path segments that locate its project directory.
///
/// `pkg:sysand/<publisher>/<name>` resolves under `<publisher>/<name>/`.
/// Any other IRI resolves under `_iri/<sha256_hex(iri)>/`.
fn iri_path_segments(iri: &str) -> Vec<String> {
    if let Some((publisher, name)) = parse_pkg_sysand_iri(iri) {
        return vec![publisher.to_string(), name.to_string()];
    }

    let hash = segment_uri_generic::<_, Sha256>(iri)
        .next()
        .expect("segment_uri_generic always yields one segment");
    vec![IRI_HASH_SEGMENT.to_string(), hash]
}

impl<Policy: HTTPAuthentication> HTTPIndexEnvironmentAsync<Policy> {
    /// Return `base_url` with a guaranteed trailing slash on its path, so that
    /// `Url::join` treats the base as a directory.
    pub fn root_url(&self) -> url::Url {
        let mut result = self.base_url.clone();

        if result.path().is_empty() {
            result.set_path("/");
        } else if !result.path().ends_with('/') {
            let new_path = format!("{}/", result.path());
            result.set_path(&new_path);
        }

        result
    }

    fn url_join(url: &url::Url, join: &str) -> Result<url::Url, HTTPIndexEnvironmentError> {
        url.join(join)
            .map_err(|e| HTTPIndexEnvironmentError::JoinURL(url.as_str().into(), join.into(), e))
    }

    pub fn index_url(&self) -> Result<url::Url, HTTPIndexEnvironmentError> {
        Self::url_join(&self.root_url(), INDEX_PATH)
    }

    pub fn project_url<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        let mut result = self.root_url();
        for mut segment in iri_path_segments(iri.as_ref()) {
            segment.push('/');
            result = Self::url_join(&result, &segment)?;
        }
        Ok(result)
    }

    /// Per-version directory URL ending with a trailing slash, so that
    /// `Url::join` treats it as a directory when composing leaf URLs
    /// (`project.kpar`, `.project.json`, `.meta.json`).
    pub fn version_dir_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        let base = self.project_url(iri)?;
        Self::url_join(&base, &format!("{}/", version.as_ref()))
    }

    pub fn kpar_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, KPAR_FILE)
    }

    pub fn project_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, PROJECT_JSON_FILE)
    }

    pub fn meta_json_url<S: AsRef<str>, T: AsRef<str>>(
        &self,
        iri: S,
        version: T,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        Self::url_join(&self.version_dir_url(iri, version)?, META_JSON_FILE)
    }

    pub fn versions_url<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<url::Url, HTTPIndexEnvironmentError> {
        let base = self.project_url(iri)?;
        Self::url_join(&base, VERSIONS_PATH)
    }

    async fn fetch_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        url: url::Url,
    ) -> Result<Option<T>, HTTPIndexEnvironmentError> {
        let response = self
            .auth_policy
            .with_authentication(&self.client, &json_get_request(url.clone()))
            .await?;

        // Treat missing files as absent so that resolver chains can keep
        // trying other sources. Non-404 non-success statuses (5xx, auth, etc.)
        // surface as errors.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            return Err(HTTPIndexEnvironmentError::BadHttpStatus {
                url: url.as_str().into(),
                status: response.status(),
            });
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| HTTPIndexEnvironmentError::HttpIo(io::Error::other(e)))?;

        serde_json::from_slice::<T>(&bytes)
            .map(Some)
            .map_err(|source| HTTPIndexEnvironmentError::JsonParse {
                url: url.as_str().into(),
                source,
            })
    }

    async fn fetch_index(&self) -> Result<IndexJson, HTTPIndexEnvironmentError> {
        let url = self.index_url()?;
        match self.fetch_json::<IndexJson>(url.clone()).await? {
            Some(index) => Ok(index),
            None => {
                // 404 on the index root is almost always a misconfigured base
                // URL. Continuing with an empty index keeps resolver chains
                // usable, but it would otherwise be hard to diagnose — the
                // later "no resolver was able to resolve the IRI" error
                // points at the IRI, not the missing root document.
                log::warn!(
                    "{url} returned 404; treating the index as empty (check the configured index base URL)"
                );
                Ok(IndexJson { projects: vec![] })
            }
        }
    }

    /// Fetch and cache `versions.json` for the given IRI. Within a single env
    /// lifetime the document is fetched at most once per IRI — concurrent
    /// callers share a single in-flight fetch via a per-IRI `OnceCell` and
    /// later callers see the same cached `Option<Arc<...>>` (so both hits and
    /// 404s are deduplicated).
    ///
    /// The cached `VersionsJson` is **validated and sorted newest-first**:
    /// every entry's `version` is parsed with `semver::Version` (a parse
    /// failure rejects the document with `InvalidSemverVersion`), and the
    /// entries are sorted by that parsed version in descending order.
    /// Downstream code relies on the cached invariant — `versions_async`
    /// streams entries in newest-first order, and `get_project_async`'s
    /// linear scan is `O(n)` regardless.
    async fn fetch_versions_json<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<Option<Arc<VersionsJson>>, HTTPIndexEnvironmentError> {
        let iri_key = iri.as_ref();
        let cell = {
            let mut cache = self
                .versions_cache
                .lock()
                .expect("versions_cache mutex poisoned");
            Arc::clone(
                cache
                    .entry(iri_key.to_string())
                    .or_insert_with(|| Arc::new(OnceCell::new())),
            )
        };

        let cached = cell
            .get_or_try_init(|| async {
                let url = self.versions_url(iri_key)?;
                let parsed = self.fetch_json::<VersionsJson>(url.clone()).await?;
                let validated = match parsed {
                    Some(vs) => Some(Arc::new(validate_and_sort_versions(&url, vs)?)),
                    None => None,
                };
                Ok::<_, HTTPIndexEnvironmentError>(validated)
            })
            .await?;

        Ok(cached.clone())
    }
}

/// Parse every entry's `version` field with `semver::Version` and rebuild
/// the document with entries sorted newest-first by that parsed version.
/// A non-semver `version` rejects the whole document.
fn validate_and_sort_versions(
    url: &url::Url,
    vs: VersionsJson,
) -> Result<VersionsJson, HTTPIndexEnvironmentError> {
    let mut paired: Vec<(Version, VersionEntry)> = vs
        .versions
        .into_iter()
        .map(|entry| {
            let parsed = Version::parse(&entry.version).map_err(|source| {
                HTTPIndexEnvironmentError::InvalidSemverVersion {
                    url: url.as_str().into(),
                    value: entry.version.clone(),
                    source,
                }
            })?;
            Ok::<_, HTTPIndexEnvironmentError>((parsed, entry))
        })
        .collect::<Result<_, _>>()?;
    paired.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(VersionsJson {
        versions: paired.into_iter().map(|(_, e)| e).collect(),
    })
}

type ResultStream<T> =
    futures::stream::Iter<std::vec::IntoIter<Result<T, HTTPIndexEnvironmentError>>>;

impl<Policy: HTTPAuthentication> ReadEnvironmentAsync for HTTPIndexEnvironmentAsync<Policy> {
    type ReadError = HTTPIndexEnvironmentError;

    type UriStream = ResultStream<String>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let index = self.fetch_index().await?;
        let items: Vec<Result<String, HTTPIndexEnvironmentError>> =
            index.projects.into_iter().map(|p| Ok(p.iri)).collect();
        Ok(futures::stream::iter(items))
    }

    type VersionStream = ResultStream<String>;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        let versions: Vec<Result<String, HTTPIndexEnvironmentError>> =
            match self.fetch_versions_json(uri.as_ref()).await? {
                Some(vs) => vs.versions.iter().map(|e| Ok(e.version.clone())).collect(),
                None => vec![],
            };

        Ok(futures::stream::iter(versions))
    }

    type InterchangeProjectRead = IndexedRemoteProject<Policy>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let kpar_url = self.kpar_url(&uri, &version)?;
        let project_json_url = self.project_json_url(&uri, &version)?;
        let meta_json_url = self.meta_json_url(&uri, &version)?;
        let versions_url = self.versions_url(uri.as_ref())?;

        // `versions.json` is the source of truth for version selection: a 404
        // on the document, or a parsed document that doesn't list the
        // requested version, surfaces as a hard error. Once the caller has
        // asked for one concrete version, return a project wrapper seeded with
        // the inline solver data from `versions.json` plus the per-version
        // `.project.json` / `.meta.json` URLs. Those files are fetched lazily
        // only when the selected version is materialized for locking.
        let versions_json = self
            .fetch_versions_json(uri.as_ref())
            .await?
            .ok_or_else(|| HTTPIndexEnvironmentError::BadHttpStatus {
                url: versions_url.as_str().into(),
                status: reqwest::StatusCode::NOT_FOUND,
            })?;
        let entry = versions_json
            .versions
            .iter()
            .find(|e| e.version == version.as_ref())
            .cloned()
            .ok_or_else(|| HTTPIndexEnvironmentError::VersionNotInIndex {
                url: versions_url.as_str().into(),
                version: version.as_ref().to_string(),
            })?;

        let project_digest_hex = parse_sha256_digest(&entry.project_digest).ok_or_else(|| {
            HTTPIndexEnvironmentError::InvalidVersionEntry {
                url: versions_url.as_str().into(),
                version: entry.version.clone(),
                field: "project_digest",
                value: entry.project_digest.clone(),
            }
        })?;
        let kpar_digest_hex = parse_sha256_digest(&entry.kpar_digest).ok_or_else(|| {
            HTTPIndexEnvironmentError::InvalidVersionEntry {
                url: versions_url.as_str().into(),
                version: entry.version.clone(),
                field: "kpar_digest",
                value: entry.kpar_digest.clone(),
            }
        })?;
        let project = IndexedRemoteProject::new(
            kpar_url,
            self.client.clone(),
            self.auth_policy.clone(),
            entry.version.clone(),
            entry.usage.clone(),
            project_json_url,
            meta_json_url,
            entry.kpar_size,
            project_digest_hex,
            kpar_digest_hex,
        )
        .map_err(Box::new)?;

        Ok(project)
    }
}

#[cfg(test)]
#[path = "./reqwest_index_http_tests.rs"]
mod tests;
