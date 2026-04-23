// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP client for the sysand index protocol. See
//! `docs/src/index-protocol.md` for the wire format and the authority
//! split between `versions.json`, per-version `.project.json`/`.meta.json`,
//! and the kpar archive; this module is the client-side implementation.
//!
//! Protocol assumptions relied on throughout this module (stated once
//! here so per-item docs can focus on specifics):
//!
//! - `versions.json` is the per-IRI catalog; the five per-entry fields
//!   (`version`, `usage`, `project_digest`, `kpar_size`, `kpar_digest`)
//!   are all required, and entries MUST appear in **descending semver
//!   precedence**. The client validates the ordering at ingest on the
//!   parsed `semver::Version` (not lexically) and does not re-sort —
//!   downstream code relies on newest-first.
//! - Every digest is `sha256:<64 lowercase hex>`; any other shape is a
//!   protocol violation and rejects the whole `versions.json`.
//! - A 404 on any required document is a hard error. At the
//!   resolver-facing `versions_async` boundary the 404 is converted to
//!   an empty stream (so a misconfigured mirror does not block other
//!   sources), but every other caller — including `get_project_async`
//!   and `index.json` — propagates it.
//! - Forward compatibility: no type here sets
//!   `#[serde(deny_unknown_fields)]`; servers may add new optional
//!   fields, and any future schema-version signal should be added in
//!   one place rather than duplicated across documents.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use semver::Version;
use serde::Deserialize;
use sha2::Sha256;
use thiserror::Error;
use tokio::sync::OnceCell;

use crate::{
    auth::{HTTPAuthentication, StandardHTTPAuthentication},
    env::{
        AsSyncEnvironmentTokio, ReadEnvironmentAsync,
        discovery::{DiscoveryError, EndpointsCell, ResolvedEndpoints, fetch_index_config},
        iri_normalize::normalize_iri_for_hash,
        segment_uri_generic,
    },
    model::InterchangeProjectUsageRaw,
    project::index_entry::{IndexEntryProject, IndexEntryProjectError},
    purl::{SysandPurlError, parse_sysand_purl},
    resolve::net_utils::json_get_request,
};

const INDEX_PATH: &str = "index.json";
const VERSIONS_PATH: &str = "versions.json";
const KPAR_FILE: &str = "project.kpar";
const PROJECT_JSON_FILE: &str = ".project.json";
const META_JSON_FILE: &str = ".meta.json";
const IRI_HASH_SEGMENT: &str = "_iri";

/// Blocking wrapper around [`IndexEnvironmentAsync`] that drives the
/// async implementation on a Tokio runtime. Use this from synchronous call
/// sites (e.g. the CLI) where an `Environment`/`ReadEnvironment` is required;
/// all real HTTP work happens on the wrapped async implementation. The type
/// parameter is fixed to [`StandardHTTPAuthentication`] — construct the
/// async form directly if a custom auth policy is needed.
pub type IndexEnvironment =
    AsSyncEnvironmentTokio<IndexEnvironmentAsync<StandardHTTPAuthentication>>;

/// Async HTTP client for the sysand index protocol. This is the
/// authoritative implementation; [`IndexEnvironment`] is just a
/// blocking wrapper around it.
///
/// Resolves IRIs as follows:
///
/// - `pkg:sysand/<publisher>/<name>` (two valid, normalized segments) ->
///   `<publisher>/<name>/` under `index_root`.
/// - Any IRI starting with `pkg:sysand/` that is *not* well-formed and
///   normalized -> hard error
///   ([`IndexEnvironmentError::MalformedSysandPurl`]). The prefix is
///   strong enough intent that silently rerouting to `_iri/<sha256>/` would
///   mask user errors (typo, unnormalized casing, wrong segment count) as
///   "not found" — see `parse_sysand_purl` in `crate::purl`.
/// - Any other IRI -> `_iri/<sha256_hex(normalized_iri)>/` under
///   `index_root`. The IRI is first normalized — RFC 3986 §6.2.2
///   syntax-based normalization plus IDN → Punycode and an HTTP(S)
///   empty-path → `/` fixup; see [`crate::env::iri_normalize`]. The `_iri`
///   route is reserved for non-`pkg:sysand` schemes; a `pkg:sysand` IRI
///   never reaches it.
///
/// `index_root` is resolved lazily via `sysand-index-config.json` on
/// first use. The `discovery_root` the caller supplies is the URL the
/// user configured; an optional discovery document remaps it to the
/// actual `index_root` (and, independently, to the `api_root` used by
/// `publish`).
///
/// A per-IRI `versions.json` document holds the advertised versions plus
/// the five per-entry fields (`version`, `usage`, `project_digest`,
/// `kpar_size`, `kpar_digest`) needed to enumerate candidates and verify
/// later-materialized archives without downloading anything heavier.
/// Fetched documents are validated (semver + digest shape + ordering)
/// and cached in `versions_cache` so concurrent solver paths share a
/// single fetch.
#[derive(Debug)]
pub struct IndexEnvironmentAsync<Policy> {
    pub(crate) client: reqwest_middleware::ClientWithMiddleware,
    pub(crate) auth_policy: Arc<Policy>,
    /// Discovery root — the URL the user configured. The client fetches
    /// `<discovery_root>/sysand-index-config.json` on first use to
    /// resolve the `index_root` and `api_root`.
    pub(crate) discovery_root: reqwest::Url,
    /// Lazily-resolved `(index_root, api_root)` pair. The first async
    /// entry point that needs a URL triggers discovery; subsequent calls
    /// share the cached result. Errors are NOT cached — a transient 5xx
    /// on the discovery endpoint is retryable within the same env
    /// lifetime.
    pub(crate) resolved: EndpointsCell,
    /// Intra-run cache of parsed `versions.json` documents, keyed by IRI.
    /// Avoids the duplicate fetches that otherwise occur because
    /// `versions_async` and `get_project_async` each independently hit the
    /// endpoint (and `get_project_async` is called once per candidate
    /// during solving). Each entry is a per-IRI `OnceCell`, so concurrent
    /// callers requesting the same IRI share a single fetch even under a
    /// parallel solver.
    ///
    /// Scoped to one env lifetime; never invalidates. There is no
    /// freshness signal in the protocol, and a single `sysand lock` run
    /// should see a stable view of the index. Cached values are already
    /// validated (see [`validate_versions`]); the raw wire form is not
    /// retained.
    pub(crate) versions_cache: Mutex<HashMap<String, VersionsCacheEntry>>,
}

/// Per-IRI cache slot: a `OnceCell` shared by all concurrent callers
/// requesting the same IRI, holding the validated `versions.json` entries.
/// The cell never stores a "project missing" value — `fetch_versions_json`
/// propagates 404 as [`HttpFetchError::BadHttpStatus`] instead, and
/// `OnceCell::get_or_try_init` discards `Err`, so transport and
/// validation errors are likewise not cached.
pub(crate) type VersionsCacheEntry = Arc<OnceCell<Arc<Vec<AdvertisedVersion>>>>;

/// A validated sha256 hex digest — 64 lowercase hex characters, with the
/// `"sha256:"` prefix already stripped. Constructed via `TryFrom<&str>`
/// which performs the only digest validation pass in this module: both
/// ingest (`validate_versions`) and every downstream site use the
/// result of that parse, so there is no second "is this hex?" check hiding
/// at the point of use. Uppercase hex is rejected rather than normalized —
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
        Ok(Sha256HexDigest(hex.to_string()))
    }
}

/// A `versions.json` entry after ingest-time validation: `version` is parsed
/// into `semver::Version`, digests are validated lowercase hex without the
/// wire prefix, and `usage`/`kpar_size` are carried through verbatim. This
/// is the representation the cache stores and the rest of the crate sees —
/// the raw [`VersionEntry`] is only alive briefly during deserialization.
#[derive(Debug, Clone)]
pub(crate) struct AdvertisedVersion {
    pub(crate) version: semver::Version,
    pub(crate) usage: Vec<InterchangeProjectUsageRaw>,
    pub(crate) project_digest: Sha256HexDigest,
    pub(crate) kpar_size: u64,
    pub(crate) kpar_digest: Sha256HexDigest,
}

#[derive(Error, Debug)]
pub enum IndexEnvironmentError {
    #[error("failed to extend URL `{0}` with path `{1}`: {2}")]
    JoinURL(Box<str>, String, url::ParseError),
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error(transparent)]
    Fetch(#[from] HttpFetchError),
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
    #[error(
        "versions.json at `{url}` has entry with version `{value}` carrying build metadata; \
         build metadata is not permitted in index-advertised versions"
    )]
    VersionHasBuildMetadata { url: Box<str>, value: String },
    #[error(
        "versions.json at `{url}` is not in descending semver precedence: \
         `{prev}` precedes `{curr}` in wire order"
    )]
    VersionsOutOfOrder {
        url: Box<str>,
        prev: String,
        curr: String,
    },
    #[error("versions.json at `{url}` does not list version `{version}`")]
    VersionNotInIndex { url: Box<str>, version: String },
    #[error("versions.json at `{url}` lists version `{version}` more than once")]
    DuplicateVersion { url: Box<str>, version: String },
    #[error("malformed `pkg:sysand` IRI `{iri}`: {source}")]
    MalformedSysandPurl {
        iri: String,
        #[source]
        source: SysandPurlError,
    },
    #[error("cannot canonicalize IRI `{iri}` for `_iri` bucket: {source}")]
    MalformedIri {
        iri: String,
        #[source]
        source: super::iri_normalize::IriNormalizeError,
    },
    #[error(transparent)]
    Project(#[from] Box<IndexEntryProjectError>),
}

/// Shared error surface for both env-level and project-level JSON fetches.
/// One `HttpFetchError` represents "something went wrong fetching a JSON
/// doc over HTTP" regardless of which caller issued the request, with
/// variants covering request dispatch, non-2xx status, body read, and
/// JSON parse failures.
#[derive(Error, Debug)]
pub enum HttpFetchError {
    #[error("HTTP request to `{url}` failed: {source}")]
    Request {
        url: Box<str>,
        #[source]
        source: reqwest_middleware::Error,
    },
    #[error("HTTP request to `{url}` returned status {status}")]
    BadHttpStatus {
        url: Box<str>,
        status: reqwest::StatusCode,
    },
    #[error("failed to read HTTP response body from `{url}`: {source}")]
    Body {
        url: Box<str>,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to parse JSON from `{url}`: {source}")]
    JsonParse {
        url: Box<str>,
        #[source]
        source: serde_json::Error,
    },
}

/// Whether a 404 on the requested URL is a successful "no such document"
/// signal (`AllowNotFound` — e.g. an optional `versions.json`) or a hard
/// error (`RequirePresent` — e.g. the per-version `.project.json` that must
/// exist once a version has been selected). This is the only policy
/// difference between the two callers of [`fetch_json`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingPolicy {
    AllowNotFound,
    RequirePresent,
}

/// Fetch and JSON-parse one document from `url` through `client`+`auth`. A
/// 404 returns `Ok(None)` under [`MissingPolicy::AllowNotFound`] and a
/// [`HttpFetchError::BadHttpStatus`] under [`MissingPolicy::RequirePresent`];
/// any other non-success status is always an error.
pub(crate) async fn fetch_json<T: for<'de> serde::Deserialize<'de>, P: HTTPAuthentication>(
    client: &reqwest_middleware::ClientWithMiddleware,
    auth: &P,
    url: &url::Url,
    missing: MissingPolicy,
) -> Result<Option<T>, HttpFetchError> {
    let response = auth
        .with_authentication(client, &json_get_request(url.clone()))
        .await
        .map_err(|source| HttpFetchError::Request {
            url: url.as_str().into(),
            source,
        })?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND && missing == MissingPolicy::AllowNotFound {
        return Ok(None);
    }

    if !status.is_success() {
        return Err(HttpFetchError::BadHttpStatus {
            url: url.as_str().into(),
            status,
        });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|source| HttpFetchError::Body {
            url: url.as_str().into(),
            source,
        })?;

    serde_json::from_slice::<T>(&bytes)
        .map(Some)
        .map_err(|source| HttpFetchError::JsonParse {
            url: url.as_str().into(),
            source,
        })
}

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

/// Per-project `versions.json`: enough to enumerate candidates and
/// verify archives without downloading first. The publish-time artifact
/// metadata (`project_digest`, `kpar_size`, `kpar_digest`) lets the
/// client populate the lockfile lazily; `.project.json` / `.meta.json`
/// are only fetched once a specific version is materialized, and the
/// client reconciles them against these digests before exposing either.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VersionsJson {
    versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct VersionEntry {
    version: String,
    /// Required so the solver can run on `versions.json` alone, without
    /// fetching each candidate's `.project.json`.
    usage: Vec<InterchangeProjectUsageRaw>,
    /// Canonical project digest (sha256 over canonicalized info+meta),
    /// used to populate the lockfile checksum without downloading the kpar.
    project_digest: String,
    /// Byte length of the kpar archive; lets `sources_async` skip a HEAD.
    kpar_size: u64,
    /// Digest of the kpar archive bytes, verified against the streamed
    /// body when the archive is downloaded.
    kpar_digest: String,
}

/// Map an IRI to the index path segments that locate its project directory.
///
/// - `pkg:sysand/<publisher>/<name>` (well-formed and normalized) resolves
///   under `<publisher>/<name>/`.
/// - Any IRI starting with `pkg:sysand/` that fails parsing/normalization is
///   rejected as [`IndexEnvironmentError::MalformedSysandPurl`].
/// - Any other IRI resolves under `_iri/<sha256_hex(canonical_iri)>/`,
///   where the IRI is first canonicalized — syntax-based normalization
///   via [`fluent_uri::Iri::normalize`], IDN → Punycode on non-ASCII
///   RegName hosts, and an HTTP(S) empty-path → `/` fixup. The `_iri`
///   route never serves a `pkg:sysand` IRI: erroring loudly is
///   preferable to silently routing typos / non-canonical casing to a
///   hash bucket where they can never resolve.
fn iri_path_segments(iri: &str) -> Result<Vec<String>, IndexEnvironmentError> {
    match parse_sysand_purl(iri) {
        Ok(Some((publisher, name))) => Ok(vec![publisher.to_string(), name.to_string()]),
        Ok(None) => {
            let normalized = normalize_iri_for_hash(iri).map_err(|source| {
                IndexEnvironmentError::MalformedIri {
                    iri: iri.to_string(),
                    source,
                }
            })?;
            let hash = segment_uri_generic::<_, Sha256>(&normalized)
                .next()
                .expect("segment_uri_generic always yields one segment");
            Ok(vec![IRI_HASH_SEGMENT.to_string(), hash])
        }
        Err(source) => Err(IndexEnvironmentError::MalformedSysandPurl {
            iri: iri.to_string(),
            source,
        }),
    }
}

/// URL-building helpers that operate on a resolved `(index_root, api_root)`
/// pair. Kept sync + self-contained so per-IRI URL construction doesn't
/// need to re-traverse discovery on every call — the async wrappers on
/// [`IndexEnvironmentAsync`] resolve endpoints once and then delegate here.
impl ResolvedEndpoints {
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
        let mut result = self.index_root.clone();
        for mut segment in iri_path_segments(iri.as_ref())? {
            segment.push('/');
            result = Self::url_join(&result, &segment)?;
        }
        Ok(result)
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

impl<Policy: HTTPAuthentication> IndexEnvironmentAsync<Policy> {
    /// Resolve (once per env lifetime) the `(index_root, api_root)` pair
    /// from the discovery root. On first use the client fetches
    /// `sysand-index-config.json` and extracts the two roots; absent
    /// fields default to the discovery root. Transient failures are not
    /// cached — retries can proceed within the same env.
    pub(crate) async fn endpoints(&self) -> Result<&ResolvedEndpoints, IndexEnvironmentError> {
        Ok(self
            .resolved
            .get_or_try_init(|| async {
                fetch_index_config(&self.client, &*self.auth_policy, &self.discovery_root).await
            })
            .await?)
    }

    async fn fetch_optional_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        url: &url::Url,
    ) -> Result<Option<T>, IndexEnvironmentError> {
        Ok(fetch_json(
            &self.client,
            &*self.auth_policy,
            url,
            MissingPolicy::AllowNotFound,
        )
        .await?)
    }

    async fn fetch_index(&self) -> Result<IndexJson, IndexEnvironmentError> {
        // Propagate a 404 as a hard error: empty-but-live indices serve
        // `{"projects": []}` with 200 OK, so 404 really means "this URL
        // is not a sysand index". Surfacing that lets the resolver
        // chain give up on this source with a signal that points at the
        // actual problem.
        let url = self.endpoints().await?.index_url()?;
        self.fetch_optional_json::<IndexJson>(&url)
            .await?
            .ok_or_else(|| {
                IndexEnvironmentError::Fetch(HttpFetchError::BadHttpStatus {
                    url: url.as_str().into(),
                    status: reqwest::StatusCode::NOT_FOUND,
                })
            })
    }

    /// Fetch, validate, and cache `versions.json` for `iri`. Successful
    /// fetches are served once per IRI via a per-IRI `OnceCell`; failures
    /// (transport, validation, 404) are not cached — retries re-fetch and
    /// re-validate. See [`validate_versions`] for the ingest checks.
    async fn fetch_versions_json<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<Arc<Vec<AdvertisedVersion>>, IndexEnvironmentError> {
        let iri_key = iri.as_ref();
        let cell = {
            // Recover from a poisoned mutex rather than panicking the whole
            // process: the critical section only inserts a fresh `OnceCell`
            // into the hashmap, so a poisoned slot just reflects a panic
            // that happened with the lock held — the data read back from
            // it is well-formed.
            let mut cache = self
                .versions_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                cache
                    .entry(iri_key.to_string())
                    .or_insert_with(|| Arc::new(OnceCell::new())),
            )
        };

        let cached = cell
            .get_or_try_init(|| async {
                let endpoints = self.endpoints().await?;
                let url = endpoints.versions_url(iri_key)?;
                let parsed: VersionsJson = fetch_json(
                    &self.client,
                    &*self.auth_policy,
                    &url,
                    MissingPolicy::RequirePresent,
                )
                .await?
                .expect("RequirePresent never returns Ok(None)");
                Ok::<_, IndexEnvironmentError>(Arc::new(validate_versions(&url, parsed)?))
            })
            .await?;

        Ok(cached.clone())
    }
}

/// Validate every entry's required fields (semver parse, no build
/// metadata, digest shape, uniqueness) and emit the
/// `AdvertisedVersion`s the rest of the crate consumes. The check
/// runs at fetch/cache time rather than per-materialization: the
/// cache is shared across candidate enumeration and lockfile
/// assembly, so catching a protocol violation up front avoids a
/// delayed surprise downstream.
///
/// Ordering is checked at ingest on the parsed `semver::Version`, not
/// lexically — violations surface as
/// [`IndexEnvironmentError::VersionsOutOfOrder`]. pubgrub's internal
/// sorting during solving does not mask the bug at the protocol
/// boundary.
fn validate_versions(
    url: &url::Url,
    vs: VersionsJson,
) -> Result<Vec<AdvertisedVersion>, IndexEnvironmentError> {
    let validated: Vec<AdvertisedVersion> =
        vs.versions
            .into_iter()
            .map(|entry| {
                let version = Version::parse(&entry.version).map_err(|source| {
                    IndexEnvironmentError::InvalidSemverVersion {
                        url: url.as_str().into(),
                        value: entry.version.clone(),
                        source,
                    }
                })?;
                // `semver::Version::parse` is lenient on the `+build…`
                // suffix; the wire-format rejection lives here.
                if !version.build.is_empty() {
                    return Err(IndexEnvironmentError::VersionHasBuildMetadata {
                        url: url.as_str().into(),
                        value: entry.version.clone(),
                    });
                }
                let project_digest = Sha256HexDigest::try_from(entry.project_digest.as_str())
                    .map_err(|_| IndexEnvironmentError::InvalidVersionEntry {
                        url: url.as_str().into(),
                        version: entry.version.clone(),
                        field: "project_digest",
                        value: entry.project_digest.clone(),
                    })?;
                let kpar_digest =
                    Sha256HexDigest::try_from(entry.kpar_digest.as_str()).map_err(|_| {
                        IndexEnvironmentError::InvalidVersionEntry {
                            url: url.as_str().into(),
                            version: entry.version.clone(),
                            field: "kpar_digest",
                            value: entry.kpar_digest.clone(),
                        }
                    })?;
                Ok::<_, IndexEnvironmentError>(AdvertisedVersion {
                    version,
                    usage: entry.usage,
                    project_digest,
                    kpar_size: entry.kpar_size,
                    kpar_digest,
                })
            })
            .collect::<Result<_, _>>()?;
    // "Pick the best duplicate" has no principled answer — two entries
    // with the same semver may carry different digests. Letting them
    // reach `resolve_candidates` would leak non-determinism into
    // pubgrub, so reject here.
    let mut seen = std::collections::HashSet::new();
    for v in &validated {
        if !seen.insert(v.version.clone()) {
            return Err(IndexEnvironmentError::DuplicateVersion {
                url: url.as_str().into(),
                version: v.version.to_string(),
            });
        }
    }
    // Parsed-semver ordering check — catches `10.0.0-beta.1` before
    // `10.0.0`, which lexical order would miss.
    for pair in validated.windows(2) {
        if pair[0].version.cmp(&pair[1].version) != std::cmp::Ordering::Greater {
            return Err(IndexEnvironmentError::VersionsOutOfOrder {
                url: url.as_str().into(),
                prev: pair[0].version.to_string(),
                curr: pair[1].version.to_string(),
            });
        }
    }
    Ok(validated)
}

type ResultStream<T> = futures::stream::Iter<std::vec::IntoIter<Result<T, IndexEnvironmentError>>>;

impl<Policy: HTTPAuthentication> ReadEnvironmentAsync for IndexEnvironmentAsync<Policy> {
    type ReadError = IndexEnvironmentError;

    type UriStream = ResultStream<String>;

    async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
        let index = self.fetch_index().await?;
        let items: Vec<Result<String, IndexEnvironmentError>> =
            index.projects.into_iter().map(|p| Ok(p.iri)).collect();
        Ok(futures::stream::iter(items))
    }

    type VersionStream = ResultStream<String>;

    async fn versions_async<S: AsRef<str>>(
        &self,
        uri: S,
    ) -> Result<Self::VersionStream, Self::ReadError> {
        // Downgrade a 404 to an empty stream at the resolver-chain
        // boundary (with a warning so the misconfiguration stays
        // visible) so a broken mirror doesn't block
        // `SequentialResolver` / `CombinedResolver` from trying the
        // next source. Non-404 errors propagate.
        match self.fetch_versions_json(uri.as_ref()).await {
            Ok(vs) => {
                let versions: Vec<Result<String, IndexEnvironmentError>> =
                    vs.iter().map(|e| Ok(e.version.to_string())).collect();
                Ok(futures::stream::iter(versions))
            }
            Err(IndexEnvironmentError::Fetch(HttpFetchError::BadHttpStatus { url, status }))
                if status == reqwest::StatusCode::NOT_FOUND =>
            {
                log::warn!(
                    "versions.json at `{url}` returned 404; this is a server-side \
                     protocol violation (missing mirrors should 200 an empty list). \
                     Skipping this source for `{iri}`.",
                    iri = uri.as_ref(),
                );
                Ok(futures::stream::iter(vec![]))
            }
            Err(other) => Err(other),
        }
    }

    type InterchangeProjectRead = IndexEntryProject<Policy>;

    async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
        &self,
        uri: S,
        version: T,
    ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
        // Validate the requested version against the advertised set
        // before constructing per-version leaf URLs — defense in depth
        // so a malformed `version` argument (e.g. `"../evil"`) cannot
        // be spliced into URL paths even though `Url::join` would
        // otherwise resolve such a segment relative to the project
        // base.
        let endpoints = self.endpoints().await?;
        let versions_url = endpoints.versions_url(uri.as_ref())?;
        let versions = self.fetch_versions_json(uri.as_ref()).await?;
        // Compare parsed-to-parsed so semver-equivalent but
        // non-byte-identical caller inputs match. A parse failure
        // surfaces as `VersionNotInIndex` — a non-semver string
        // can't appear in a validated `versions.json` by construction.
        let requested = Version::parse(version.as_ref()).map_err(|_| {
            IndexEnvironmentError::VersionNotInIndex {
                url: versions_url.as_str().into(),
                version: version.as_ref().to_string(),
            }
        })?;
        let advertised = versions
            .iter()
            .find(|e| e.version == requested)
            .cloned()
            .ok_or_else(|| IndexEnvironmentError::VersionNotInIndex {
                url: versions_url.as_str().into(),
                version: version.as_ref().to_string(),
            })?;

        // Build leaf URLs from the validated version (i.e. the `Display` of
        // the parsed `semver::Version`), not the caller-supplied string.
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
}

#[cfg(test)]
#[path = "./index_tests.rs"]
mod tests;
