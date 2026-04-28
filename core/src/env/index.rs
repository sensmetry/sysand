// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! HTTP client for the sysand index protocol. See
//! `docs/src/index-protocol.md` for the wire format and the authority
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
//! - The wire contract is kept in `docs/src/index-protocol.md`; the code
//!   does not use `#[serde(deny_unknown_fields)]` so new optional fields are
//!   ignored by default.

use std::{
    collections::{HashMap, hash_map::Entry},
    rc::Rc,
    sync::Arc,
};

use semver::Version;
use serde::{Deserialize, de::DeserializeOwned};
use sha2::Sha256;
use thiserror::Error;

use crate::{
    auth::HTTPAuthentication,
    env::{
        ReadEnvironmentAsync,
        discovery::{DiscoveryError, ResolvedEndpoints, fetch_index_config},
        iri_normalize::normalize_iri_for_hash,
        segment_uri_generic,
    },
    model::InterchangeProjectUsageRaw,
    project::index_entry::{IndexEntryProject, IndexEntryProjectError},
    purl::{SysandPurlError, parse_sysand_purl},
    resolve::net_utils::json_get_request,
};

const IRI_HASH_SEGMENT: &str = "_iri";

/// Async HTTP client for the sysand index protocol.
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
/// and cached in `versions_cache` so later reads in the same run reuse
/// the parsed result.
#[derive(Debug)]
pub struct IndexEnvironmentAsync<Policy> {
    client: reqwest_middleware::ClientWithMiddleware,
    auth_policy: Arc<Policy>,
    /// User-configured discovery root. When present, `endpoints` is
    /// populated lazily from `<discovery_root>/sysand-index-config.json`
    /// on first actual index access.
    discovery_root: Option<url::Url>,
    /// Resolved `(index_root, api_root)` pair. Test callers may seed this
    /// at construction; production index resolvers leave it empty until
    /// the index is actually queried.
    endpoints: tokio::sync::OnceCell<ResolvedEndpoints>,
    /// Intra-run cache of parsed `versions.json` documents, keyed by IRI.
    /// Avoids the duplicate fetches that otherwise occur because
    /// `versions_async` and `get_project_async` each independently hit the
    /// endpoint (and `get_project_async` is called once per candidate
    /// during solving).
    ///
    /// Scoped to one env lifetime; never invalidates. There is no
    /// freshness signal in the protocol, and a single `sysand lock` run
    /// should see a stable view of the index. Cached values are already
    /// validated (see [`validate_versions`]); the raw wire form is not
    /// retained. Transport and validation errors are not cached —
    /// retries re-fetch.
    ///
    // This is a Mutex to enable caching in &self methods of ReadEnvironment
    // trait.
    versions_cache: tokio::sync::Mutex<HashMap<String, VersionsCacheEntry>>,
}

impl<Policy> IndexEnvironmentAsync<Policy> {
    /// Build an env from already-resolved `endpoints`. Tests construct
    /// `ResolvedEndpoints` directly when they don't need to exercise the
    /// discovery flow; production resolver construction should prefer
    /// [`Self::from_discovery_root`] so network discovery stays lazy.
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
            versions_cache: Default::default(),
        }
    }

    /// Build an env from a user-configured discovery root without
    /// contacting the network. The discovery document is fetched once,
    /// lazily, when a caller first enumerates or materializes projects
    /// from this index.
    pub fn from_discovery_root(
        client: reqwest_middleware::ClientWithMiddleware,
        auth_policy: Arc<Policy>,
        discovery_root: url::Url,
    ) -> Self {
        Self {
            client,
            auth_policy,
            discovery_root: Some(discovery_root),
            endpoints: tokio::sync::OnceCell::new(),
            versions_cache: Default::default(),
        }
    }
}

/// Per-IRI cache slot: validated `versions.json` entries, shared across
/// callers by `Rc`. The slot is only populated on a successful fetch
/// (see `fetch_versions_json`); validation errors propagate, and the
/// "project not in this index" 404 outcome (§8) is intentionally not
/// cached so a later publish to the same index becomes visible without
/// restart.
pub(crate) type VersionsCacheEntry = Rc<[AdvertisedVersion]>;

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
    pub(crate) status: Status,
}

/// Retirement state of a `versions.json` entry. See §8 of the index
/// protocol for the wire contract and §11 for the transition rules
/// (`Available → Yanked`, `Available → Removed`, `Yanked → Removed`;
/// no other transitions). An omitted `status` in the JSON parses as
/// [`Status::Available`] — the field is optional for forward
/// compatibility with indexes predating the retirement model.
///
/// Serializers SHOULD pair the `Serialize` impl with
/// `#[serde(skip_serializing_if = "Status::is_available")]` on any
/// field carrying a `Status`, so unretired entries keep their on-wire
/// shape unchanged (§8 SHOULD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    #[default]
    Available,
    Yanked,
    Removed,
}

impl Status {
    /// Predicate for `#[serde(skip_serializing_if = "...")]` so emitters
    /// drop `status` when it would round-trip as the default.
    #[allow(dead_code)]
    pub(crate) fn is_available(&self) -> bool {
        matches!(self, Status::Available)
    }
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
    #[error("project `{iri}` is not in this index (versions.json at `{url}` returned 404)")]
    ProjectNotInIndex { url: Box<str>, iri: String },
    #[error(
        "version `{version}` of `{iri}` was removed upstream (versions.json at `{url}` marks it `status: \"removed\"`)"
    )]
    VersionRemoved {
        url: Box<str>,
        iri: String,
        version: String,
    },
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
/// signal (`AllowNotFound` — e.g. `versions.json`, where 404 means the
/// project is not in this index per §8) or a hard error
/// (`RequirePresent` — e.g. the per-version `.project.json` that must
/// exist once a version has been selected). This is the only policy
/// difference between callers of [`fetch_json`].
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
    /// Retirement state (§8). Optional on the wire; an omitted field
    /// deserializes as [`Status::Available`].
    #[serde(default)]
    status: Status,
}

/// Map an IRI to the index path segments that locate its project directory.
/// The detailed wire mapping is specified in `docs/src/index-protocol.md`;
/// this function keeps malformed `pkg:sysand/...` IRIs out of the generic
/// `_iri/<hash>/` bucket so user typos fail loudly.
pub(crate) fn iri_path_segments(iri: &str) -> Result<Vec<String>, IndexEnvironmentError> {
    match parse_sysand_purl(iri) {
        Ok(Some((publisher, name))) => Ok(vec![publisher.to_string(), name.to_string()]),
        Ok(None) => {
            let malformed = |source| IndexEnvironmentError::MalformedIri {
                iri: iri.to_string(),
                source,
            };
            let parsed = fluent_uri::Iri::parse(iri)
                .map_err(|e| malformed(super::iri_normalize::IriNormalizeError::Parse(e)))?;
            let normalized = normalize_iri_for_hash(parsed).map_err(malformed)?;
            let hash = segment_uri_generic::<_, Sha256>(normalized.as_str())
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
        // Propagate a 404 as a hard error: empty-but-live indices serve
        // `{"projects": []}` with 200 OK, so 404 really means "this URL
        // is not a sysand index". A misconfigured base URL must surface
        // as a hard error rather than be silently skipped by a resolver
        // chain.
        let url = self.endpoints().await?.index_url()?;

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

    /// Fetch, validate, and cache `versions.json` for `iri`. See
    /// [`validate_versions`] for the ingest checks. Returns `Ok(None)`
    /// when the server returns 404 — per §8 that means the project is
    /// not in this index, which the callers (`versions_async`,
    /// `get_project_async`) translate into their respective absence
    /// signals. The absence outcome is intentionally not cached so a
    /// later publish to the same index is observable without restart.
    async fn fetch_versions_json<S: AsRef<str>>(
        &self,
        iri: S,
    ) -> Result<Option<Rc<[AdvertisedVersion]>>, IndexEnvironmentError> {
        let iri_key = iri.as_ref();
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
) -> Result<Rc<[AdvertisedVersion]>, IndexEnvironmentError> {
    let validated: Rc<[AdvertisedVersion]> =
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
                    status: entry.status,
                })
            })
            .collect::<Result<_, _>>()?;
    // "Pick the best duplicate" has no principled answer — two entries
    // with the same semver may carry different digests. Letting them
    // reach `resolve_candidates` would leak non-determinism into
    // pubgrub, so reject here.
    let mut seen = std::collections::HashSet::new();
    for v in validated.iter() {
        if !seen.insert(v.version.clone()) {
            return Err(IndexEnvironmentError::DuplicateVersion {
                url: url.as_str().into(),
                version: v.version.to_string(),
            });
        }
    }
    // Parsed-semver ordering check — catches `10.0.0-beta.1` before
    // `10.0.0`, which lexical order would miss.
    for [v1, v2] in validated.array_windows() {
        if v1.version <= v2.version {
            return Err(IndexEnvironmentError::VersionsOutOfOrder {
                url: url.as_str().into(),
                prev: v1.version.to_string(),
                curr: v2.version.to_string(),
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
        // §8 — a 404 on `versions.json` means the project is not in
        // this index. Yield an empty stream so `SequentialResolver` /
        // `CombinedResolver` cleanly fall through to the next source;
        // non-404 errors still propagate.
        let Some(vs) = self.fetch_versions_json(uri.as_ref()).await? else {
            log::debug!(
                "versions.json 404 for `{iri}`: project not in this index, \
                 yielding empty candidate stream",
                iri = uri.as_ref(),
            );
            return Ok(futures::stream::iter(vec![]));
        };
        // §12 — resolver-visible stream MUST exclude retired entries
        // (`yanked` / `removed`) so solve/lock can't select them for a
        // new resolution. Replaying a pinned lockfile uses
        // `get_project_async` directly, which allows `yanked` (files
        // still served) and hard-fails `removed` — see §13 and the §9
        // file-presence rule.
        let versions: Vec<Result<String, IndexEnvironmentError>> = vs
            .iter()
            .filter(|e| {
                if e.status == Status::Available {
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
        // Validate the requested version against the advertised set
        // before constructing per-version leaf URLs. We only fetch
        // versions the index has explicitly listed in `versions.json`.
        let endpoints = self.endpoints().await?;
        let versions_url = endpoints.versions_url(uri.as_ref())?;
        // §8 — a `versions.json` 404 means the project is not in this
        // index. Surface it as a distinct `ProjectNotInIndex` error so
        // direct callers can tell "not here" apart from "the index
        // spoke but doesn't list this version" (`VersionNotInIndex`)
        // or any other transport failure.
        let versions = self
            .fetch_versions_json(uri.as_ref())
            .await?
            .ok_or_else(|| IndexEnvironmentError::ProjectNotInIndex {
                url: versions_url.as_str().into(),
                iri: uri.as_ref().to_string(),
            })?;
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

        // A `removed` entry's per-version files are intentionally absent.
        // Refuse before issuing the fetch. `yanked` entries stay reachable
        // here; they are excluded from new resolutions at `versions_async`.
        if advertised.status == Status::Removed {
            return Err(IndexEnvironmentError::VersionRemoved {
                url: versions_url.as_str().into(),
                iri: uri.as_ref().to_string(),
                version: advertised.version.to_string(),
            });
        }

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
