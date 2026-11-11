// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP client for the sysand index protocol. See
//! `design/index-protocol.md` for the wire format and the authority
//! split between `versions.json`, per-version `.project.json`/`.meta.json`,
//! and the kpar archive; this module is the client-side implementation.
//!
//! Implementation notes:
//!
//! - `versions.json` entries are validated once at ingest and then cached
//!   in newest-first order.
//! - `versions.json` 404 is represented as absence from this index;
//!   `index.json` and per-version 404s remain hard errors.
//! - Retired versions are filtered at candidate enumeration, and `removed`
//!   entries are rejected before per-version files are fetched.
//! - Wire types do not use `#[serde(deny_unknown_fields)]`, so new optional
//!   fields are ignored.

use std::{
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
    sync::Arc,
};

use semver::Version;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::{
    auth::HTTPAuthentication,
    env::{
        ReadEnvironmentAsync,
        discovery::{DiscoveryError, ResolvedEndpoints, fetch_index_config},
    },
    index::{
        iri::ParseIriError,
        model::{IndexJson, ProjectStatus, VersionStatus, VersionsJson, VersionsJsonError},
    },
    index_location::IndexLocation,
    project::index_entry::{IndexEntryProject, IndexEntryProjectError},
    redact::{HttpError, RedactedUrl, ResponseUrl},
    resolve::net_utils::json_get_request,
};

use super::ProjectChecksumResult;

/// Async HTTP client for the sysand index protocol.
///
/// `index_root` is resolved lazily via `sysand-index-config.json` on
/// first use. An optional discovery document remaps the user-configured
/// `discovery_root` to the actual `index_root` and, independently, to the
/// `api_root` used by `publish`.
///
/// A per-IRI `versions.json` document holds the advertised versions plus
/// the per-entry fields (`version`, `usage`, `kpar_size`, `kpar_digest`)
/// needed to enumerate candidates and verify later-fetched project
/// metadata and archives. The optional `status` field controls retirement
/// filtering. Fetched documents are validated and cached in
/// `versions_cache`.
#[derive(Debug)]
pub struct IndexEnvironmentAsync<Policy> {
    client: reqwest_middleware::ClientWithMiddleware,
    auth_policy: Arc<Policy>,
    /// User-configured discovery root. When present, `endpoints` is
    /// populated lazily from `<discovery_root>/sysand-index-config.json`
    /// on first actual index access.
    discovery_root: Option<IndexLocation>,
    /// Resolved `(index_root, api_root)` pair. Seeded at construction by
    /// [`Self::new`], otherwise filled on first index access.
    endpoints: tokio::sync::OnceCell<ResolvedEndpoints>,
    /// Cache of parsed `versions.json` documents, keyed by IRI.
    ///
    /// Scoped to one env lifetime and never invalidated: the protocol has no
    /// freshness signal. Cached values are already validated (see
    /// [`validate_versions`]). Transport, validation, and not-found outcomes
    /// are not cached, so retries re-fetch.
    ///
    // A Mutex so `&self` methods can fill the cache.
    versions_cache: tokio::sync::Mutex<HashMap<String, VersionsCacheEntry>>,
}

impl<Policy> IndexEnvironmentAsync<Policy> {
    /// Build an env from already-resolved `endpoints`, skipping discovery.
    /// Prefer [`Self::from_discovery_root`] outside tests.
    pub fn new(
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        endpoints: ResolvedEndpoints,
    ) -> Self {
        let endpoints_cell = tokio::sync::OnceCell::new();
        endpoints_cell
            .set(endpoints)
            .expect("newly-created OnceCell accepts initial endpoints");
        Self {
            client,
            auth_policy,
            discovery_root: None,
            endpoints: endpoints_cell,
            versions_cache: Mutex::default(),
        }
    }

    /// Build an env from a user-configured discovery root without
    /// contacting the network. The discovery document is fetched once, on
    /// first enumeration or materialization.
    pub fn from_discovery_root(
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        discovery_root: IndexLocation,
    ) -> Self {
        Self {
            client,
            auth_policy,
            discovery_root: Some(discovery_root),
            endpoints: tokio::sync::OnceCell::new(),
            versions_cache: Mutex::default(),
        }
    }
}

/// Validated `versions.json` entries of one IRI, shared by `Rc`.
pub(crate) type VersionsCacheEntry = Rc<[AdvertisedVersion]>;

pub(crate) use crate::index::model::AdvertisedVersion;

#[derive(Error, Debug)]
pub enum IndexEnvironmentError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error(transparent)]
    Fetch(#[from] HttpFetchError),
    /// The `versions.json` shape rules, shared with the filesystem index.
    #[error(transparent)]
    Versions(#[from] VersionsJsonError),
    #[error("versions.json at `{url}` does not list version `{version}`")]
    VersionNotInIndex { url: RedactedUrl, version: String },
    #[error("project `{iri}` is not in this index (versions.json at `{url}` returned 404)")]
    ProjectNotInIndex { url: RedactedUrl, iri: String },
    #[error(
        "version `{version}` of `{iri}` was removed upstream (versions.json at `{url}` marks it `status: \"removed\"`)"
    )]
    VersionRemoved {
        url: RedactedUrl,
        iri: String,
        version: String,
    },
    #[error(transparent)]
    MalformedIri(#[from] ParseIriError),
    #[error(transparent)]
    Project(#[from] Box<IndexEntryProjectError>),
}

/// Failure to fetch a JSON document over HTTP: request dispatch, non-2xx
/// status, body read, or JSON parse.
#[derive(Error, Debug)]
pub enum HttpFetchError {
    #[error("HTTP request to `{url}` failed")]
    Request {
        url: RedactedUrl,
        #[source]
        source: HttpError,
    },
    #[error("HTTP request to {url} returned status {status}")]
    BadHttpStatus {
        url: ResponseUrl,
        status: reqwest::StatusCode,
    },
    #[error("failed to read HTTP response body from {url}")]
    Body {
        url: ResponseUrl,
        #[source]
        source: HttpError,
    },
    #[error("failed to parse JSON from {url}")]
    JsonParse {
        url: ResponseUrl,
        #[source]
        source: serde_json::Error,
    },
}

/// Whether a 404 on the requested URL is a successful "no such document"
/// signal (`AllowNotFound`, e.g. a missing project in the index) or a hard
/// error (`RequirePresent`, e.g. a document that must exist once a version
/// has been selected).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingPolicy {
    AllowNotFound,
    RequirePresent,
}

/// Fetch and JSON-parse one document from `url` through `client`+`auth`. A
/// 404 returns `Ok(None)` under [`MissingPolicy::AllowNotFound`] and a
/// [`HttpFetchError::BadHttpStatus`] under [`MissingPolicy::RequirePresent`];
/// any other non-success status is always an error.
pub(crate) async fn fetch_json<T: DeserializeOwned, P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    url: &url::Url,
    missing: MissingPolicy,
) -> Result<Option<T>, HttpFetchError> {
    let response = auth
        .with_authentication(client, &json_get_request(url.clone()))
        .await
        .map_err(|source| HttpFetchError::Request {
            url: url.clone().into(),
            source: source.into(),
        })?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND && missing == MissingPolicy::AllowNotFound {
        return Ok(None);
    }

    let response_url = ResponseUrl::new(url, response.url());
    if !status.is_success() {
        return Err(HttpFetchError::BadHttpStatus {
            url: response_url,
            status,
        });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|source| HttpFetchError::Body {
            url: response_url.clone(),
            source: source.into(),
        })?;

    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => Ok(Some(v)),
        Err(source) => Err(HttpFetchError::JsonParse {
            url: response_url,
            source,
        }),
    }
}

impl<Policy: HTTPAuthentication> IndexEnvironmentAsync<Policy> {
    async fn endpoints(&self) -> Result<&ResolvedEndpoints, IndexEnvironmentError> {
        if let Some(discovery_root) = self.discovery_root.as_ref() {
            return self
                .endpoints
                .get_or_try_init(|| async {
                    fetch_index_config(&self.client, &*self.auth_policy, discovery_root).await
                })
                .await
                .map_err(IndexEnvironmentError::Discovery);
        }

        Ok(self
            .endpoints
            .get()
            .expect("resolved-endpoint constructor initializes endpoints"))
    }

    async fn fetch_index(&self) -> Result<IndexJson, IndexEnvironmentError> {
        // A 404 is a hard error: an empty index serves `{"projects": []}`
        // with 200, so 404 means "this URL is not a sysand index".
        let url = self.endpoints().await?.index_url();

        match fetch_json(
            &self.client,
            &*self.auth_policy,
            &url,
            MissingPolicy::RequirePresent,
        )
        .await
        {
            Ok(json) => Ok(json.expect("RequirePresent must not return Ok(None)")),
            Err(e) => Err(e.into()),
        }
    }

    /// Fetch, validate, and cache `versions.json` for `iri`. Returns
    /// `Ok(None)` on a 404 (the project is not in this index).
    async fn fetch_versions_json<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<Option<Rc<[AdvertisedVersion]>>, IndexEnvironmentError> {
        let iri_key = iri.as_ref();
        #[expect(clippy::significant_drop_in_scrutinee, reason = "returns immediately")]
        if let Some(cached) = self.versions_cache.lock().await.get(iri_key).cloned() {
            return Ok(Some(cached));
        }

        let url = self.endpoints().await?.versions_url(iri_key)?;
        let fetched = fetch_json::<VersionsJson, _>(
            &self.client,
            &*self.auth_policy,
            &url,
            MissingPolicy::AllowNotFound,
        )
        .await?;
        let Some(parsed) = fetched else {
            return Ok(None);
        };

        let validated = validate_versions(&url, parsed)?;
        let mut cache = self.versions_cache.lock().await;
        let val = match cache.entry(iri_key.to_owned()) {
            Entry::Occupied(occupied) => Rc::clone(occupied.get()),
            Entry::Vacant(vacant) => Rc::clone(vacant.insert_entry(validated).get()),
        };
        Ok(Some(val))
    }
}

/// Validate every entry's required fields (semver parse, no build
/// metadata, digest shape, uniqueness) and emit the `AdvertisedVersion`s.
///
/// Ordering is checked on the parsed `semver::Version`, not lexically;
/// violations surface as [`VersionsJsonError::VersionsOutOfOrder`].
fn validate_versions(
    url: &url::Url,
    vs: VersionsJson,
) -> Result<Rc<[AdvertisedVersion]>, IndexEnvironmentError> {
    // The location is shown in error messages, so it is the redacted one.
    Ok(crate::index::model::validate_versions(&RedactedUrl::from(url).to_string(), vs)?.into())
}

type ResultStream<T> = futures::stream::Iter<std::vec::IntoIter<Result<T, IndexEnvironmentError>>>;

/// TODO: move index away from ReadEnvironment traits; most new methods will not be used
/// for index
impl<Policy: HTTPAuthentication> ReadEnvironmentAsync for IndexEnvironmentAsync<Policy> {
    type ReadError = IndexEnvironmentError;

    type UriStream = ResultStream<String>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let index = self.fetch_index().await?;
        let items: Vec<Result<String, IndexEnvironmentError>> = index
            .projects
            .into_iter()
            .filter(|p| p.status == ProjectStatus::Available)
            .map(|p| Ok(p.iri))
            .collect();
        Ok(futures::stream::iter(items))
    }

    type VersionStream = ResultStream<String>;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        // A 404 on `versions.json` means the project is not in this index:
        // yield an empty stream so resolution falls through to the next
        // source. Other errors propagate.
        let Some(vs) = self.fetch_versions_json(uri.as_ref()).await? else {
            log::debug!(
                "versions.json 404 for `{iri}`: project not in this index, \
                 yielding empty candidate stream",
                iri = uri.as_ref(),
            );
            return Ok(futures::stream::iter(vec![]));
        };
        // Retired entries (`yanked` / `removed`) are not offered to new
        // resolutions.
        let versions: Vec<Result<String, IndexEnvironmentError>> = vs
            .iter()
            .filter(|e| {
                if e.status == VersionStatus::Available {
                    true
                } else {
                    log::debug!(
                        "skipping retired version `{version}` of `{iri}` \
                         (status: {status:?}) during candidate enumeration",
                        version = e.version,
                        iri = uri.as_ref(),
                        status = e.status,
                    );
                    false
                }
            })
            .map(|e| Ok(e.version.to_string()))
            .collect();
        Ok(futures::stream::iter(versions))
    }

    type InterchangeProjectRead = IndexEntryProject<Policy>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        let version = version.as_ref();
        // Only versions listed in `versions.json` are fetched.
        let endpoints = self.endpoints().await?;
        let versions_url = endpoints.versions_url(uri.as_ref())?;
        // A `versions.json` 404 is `ProjectNotInIndex`, distinct from
        // `VersionNotInIndex` (the index lists the project but not this
        // version).
        let versions = self
            .fetch_versions_json(uri.as_ref())
            .await?
            .ok_or_else(|| IndexEnvironmentError::ProjectNotInIndex {
                url: versions_url.clone().into(),
                iri: uri.as_ref().to_owned(),
            })?;
        // Invalid SemVer input is `VersionNotInIndex`: validated entries
        // are always SemVer.
        let requested = Version::parse(version).map_err(|e| {
            log::debug!(
                "index: failed to parse version `{version}`:\n{e};\n\
                treating as not present in index"
            );
            IndexEnvironmentError::VersionNotInIndex {
                url: versions_url.clone().into(),
                version: version.to_owned(),
            }
        })?;
        let advertised = versions
            .iter()
            .find(|e| e.version == requested)
            .cloned()
            .ok_or_else(|| IndexEnvironmentError::VersionNotInIndex {
                url: versions_url.clone().into(),
                version: version.to_owned(),
            })?;

        // A `removed` entry's per-version files are absent, so refuse before
        // fetching. `yanked` entries stay reachable.
        if advertised.status == VersionStatus::Removed {
            return Err(IndexEnvironmentError::VersionRemoved {
                url: versions_url.clone().into(),
                iri: uri.as_ref().to_owned(),
                version: advertised.version.to_string(),
            });
        }

        // Leaf URLs use the `Display` of the parsed version.
        let advertised_version = advertised.version.to_string();
        let kpar_url = endpoints.kpar_url(&uri, &advertised_version)?;
        let project_json_url = endpoints.project_json_url(&uri, &advertised_version)?;
        let meta_json_url = endpoints.meta_json_url(&uri, &advertised_version)?;

        let project = IndexEntryProject::new(
            kpar_url,
            project_json_url,
            meta_json_url,
            advertised,
            self.client.clone(),
            self.auth_policy.clone(),
        )
        .map_err(Box::new)?;

        Ok(project)
    }

    /// Not useful for an index
    async fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
        &self,
        _uri: S,
        _version: V,
        _checksum: &crate::project::ProjectChecksum,
    ) -> Result<ProjectChecksumResult, Self::ReadError> {
        panic!()
    }
}

#[cfg(test)]
#[path = "./index_tests.rs"]
mod tests;
