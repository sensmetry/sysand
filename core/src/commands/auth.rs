// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand auth` command orchestration (the full design is in
//! design/credential-storage.md): `do_auth_login`, `do_auth_logout`,
//! `do_auth_status`, and `do_auth_whoami`, over the credential store
//! ([`LockedBlobStore`], generic over its [`BlobBackend`]).
//!
//! Library calls never prompt and never print; the login secret arrives as
//! a parameter and progress is reported through [`AuthLoginNotice`] values
//! for the host (CLI or bindings) to render. Environment credentials are
//! passed in as [`EnvCredentialEntry`] values: this module does not read
//! the process environment.

use chrono::{DateTime, Utc};
use globset::GlobBuilder;
use thiserror::Error;
use url::Url;

use serde::Deserialize;

use crate::{
    auth::{
        BearerSelection, EnvBearerAuth, GlobMap, HTTPAuthentication, Unauthenticated,
        select_bearer, stored_bearer_map_from_records,
    },
    credential_store::{
        CredentialRecord, CredentialScheme, CredentialStoreError, CredentialSubject,
        keyring_store::{BlobBackend, LockedBlobStore},
        normalize_index_key,
    },
    env::discovery::{
        DiscoveryError, INDEX_CONFIG_PATH, IndexConfigRaw, ResolvedEndpoints, fetch_index_config,
        fetch_index_config_strict, resolve_index_config,
    },
    env::index::HttpFetchError,
    index_location::{IndexLocation, IndexLocationError, is_template_syntax},
};

/// Errors from the `sysand auth` commands.
#[derive(Debug, Error)]
pub enum AuthCommandError {
    /// `logout` targeted an index with no stored credential.
    #[error("no stored credential for `{index}`")]
    NoStoredCredential { index: String },
    /// The target is not an HTTP(S) index (for example a `file://` URL).
    #[error(
        "`{url}`: not an HTTP(S) index; nothing to authenticate to \
         (use an https:// index URL)"
    )]
    NotHttpIndex { url: String },
    /// The target could not be parsed or normalized as an index URL.
    #[error("invalid index URL for credential storage: {0}")]
    InvalidIndexUrl(String),
    /// The target is a URL template with no literal prefix at a safe URL
    /// boundary (at least `scheme://authority/`), so no credential scope
    /// glob can be derived.
    #[error(
        "`{url}`: this URL template has no literal prefix at a safe URL boundary\n\
         (at least `scheme://authority/`) to scope a credential to;\n\
         use `SYSAND_CRED_*` environment variables to authenticate it"
    )]
    TemplateWithoutAnchor { url: String },
    /// Every exercised surface rejected the credential and none accepted
    /// it, so nothing was stored.
    #[error("{}", validation_rejected_message(.index, .rejected, *.read_status, *.basic_challenge))]
    ValidationRejected {
        /// The normalized index key the login targeted.
        index: String,
        /// The surfaces that exercised and rejected the credential.
        rejected: Vec<ProbeSurface>,
        /// The HTTP status of the read surface's rejection, when the read
        /// surface rejected. A 404 hedges the message: it can mean a
        /// rejected token, but also a URL with no index at all.
        read_status: Option<u16>,
        /// Whether the read surface answered with a `WWW-Authenticate:
        /// Basic` challenge: the index wants username/password, not a
        /// bearer token, and the message routes the user to
        /// `SYSAND_CRED_*` basic credentials.
        basic_challenge: bool,
    },
    /// `whoami` could not read the index configuration, so the API root
    /// (and with it the `v1/whoami` URL) could not be resolved.
    #[error(
        "could not read the index configuration for `{index}` ({error}); cannot resolve its API"
    )]
    WhoamiDiscoveryFailed { index: String, error: String },
    /// `whoami` targeted an index whose discovery configuration does not
    /// advertise `api_root`: no identity endpoint to ask. The
    /// parenthetical matters: a private index hiding its configuration
    /// resolves to this same state.
    #[error(
        "index `{index}` does not advertise an API (`api_root`) in its\n\
         discovery configuration, so there is no identity endpoint to ask\n\
         (a private index may also hide its configuration from clients it\n\
         cannot authenticate)"
    )]
    NoAdvertisedApi { index: String },
    /// More than one credential from one source matches the `v1/whoami`
    /// URL (after collapsing entries carrying the same token), so no
    /// single identity question can be asked.
    #[error(
        "{candidates} credentials from {source_name} match `{url}`; \
         refine the patterns so exactly one matches"
    )]
    AmbiguousWhoamiCredential {
        url: String,
        // Not `source`: thiserror reserves that name for error chaining.
        source_name: &'static str,
        candidates: usize,
    },
    /// No credential of either source matches the `v1/whoami` URL. The
    /// message states the bare condition; the frontend appends its own
    /// remediation (`sysand auth login`, `SYSAND_CRED_*`) from the fields.
    #[error("no credential matches `{url}`")]
    NoWhoamiCredential { url: String, index: String },
    /// The credential store failed.
    #[error(transparent)]
    Store(#[from] CredentialStoreError),
}

fn validation_rejected_message(
    index: &str,
    rejected: &[ProbeSurface],
    read_status: Option<u16>,
    basic_challenge: bool,
) -> String {
    let mut message = match rejected {
        // A read-surface 404 is hedged: it can also mean there is simply
        // no index at this URL, so the token must not be blamed outright.
        [surface] => {
            let (endpoint, status) = match surface {
                ProbeSurface::Read => ("index.json", read_status.unwrap_or(401)),
                // The API surface rejects only on 401.
                ProbeSurface::Api => ("v1/whoami", 401),
            };
            let hedge = if *surface == ProbeSurface::Read && status == 404 {
                ", or no index exists at this URL"
            } else {
                ""
            };
            format!(
                "the index rejected the token for `{index}` \
                 (`{endpoint}` answered HTTP {status}){hedge}; nothing was stored"
            )
        }
        _ => {
            let surfaces: Vec<&str> = rejected
                .iter()
                .map(|surface| match surface {
                    ProbeSurface::Read => "the index read surface (`index.json`)",
                    ProbeSurface::Api => "the index API (`v1/whoami`)",
                })
                .collect();
            format!(
                "credential for `{index}` was rejected by {} and accepted by no surface; \
                 nothing was stored",
                surfaces.join(" and ")
            )
        }
    };
    if basic_challenge {
        // Keep this basic-auth routing hint consistent with the
        // stored-anyway variant in sysand/src/commands/auth.rs
        // (render_login_notice).
        message.push_str(
            "\nthis index uses username/password (HTTP basic) authentication; configure \
             `SYSAND_CRED_<X>_BASIC_USER` / `SYSAND_CRED_<X>_BASIC_PASS` environment \
             variables instead",
        );
    }
    message
}

/// One `SYSAND_CRED_*` environment credential, as seen by `auth status`:
/// the full variable name carrying the URL pattern, and the pattern value.
/// Never the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvCredentialEntry {
    /// The environment variable name, for example `SYSAND_CRED_TEAMIDX`.
    pub label: String,
    /// The URL glob pattern the variable holds.
    pub pattern: String,
    /// Whether this entry applies to the default index passed to
    /// [`assemble_auth_status`]: its pattern matches the default index
    /// root URL. Callers construct entries with `false`; status assembly
    /// sets it.
    pub applies_to_default: bool,
}

/// A credential-probing surface of an index: the read surface
/// (`index_root/index.json`) or the API
/// surface (`api_root/v1/whoami`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSurface {
    Read,
    Api,
}

impl std::fmt::Display for ProbeSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeSurface::Read => write!(f, "read"),
            ProbeSurface::Api => write!(f, "api"),
        }
    }
}

/// Status of one stored credential, as shown by `auth status`. Never contains
/// the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCredentialStatus {
    /// The normalized index key, in the exact form
    /// `sysand auth logout <key>` accepts.
    pub key: String,
    /// The URL glob patterns the credential applies to.
    pub globs: Vec<String>,
    /// Expiry, when a validating login learned it.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether `expires_at` is known and in the past.
    pub expired: bool,
    /// Whole days until `expires_at`, against the assembly clock. Present
    /// exactly when `expires_at` is; negative once expired.
    pub expires_in_days: Option<i64>,
    /// Who the credential authenticates as, when a validating login
    /// learned it from `v1/whoami`.
    pub subject: Option<CredentialSubject>,
    /// The token's non-secret display prefix, when a validating login
    /// learned it.
    pub token_prefix: Option<String>,
    /// The surfaces that exercised and accepted the credential at login
    /// (`"read"`, `"api"`, in probe order). Empty means nothing
    /// exercised the credential ("stored, not validated").
    pub validated: Vec<String>,
    /// Labels of `SYSAND_CRED_*` entries that may shadow this login.
    /// Approximate: an entry is listed when its pattern matches this
    /// record's key URL, but precedence is per request URL, so an env
    /// pattern can shadow requests without being listed here (more so for
    /// template keys, whose placeholder text no request URL contains).
    pub shadowed_by: Vec<String>,
    /// Whether this entry applies to the default index passed to
    /// [`assemble_auth_status`]: key equality, or a glob matching the
    /// default index root URL (an entry can cover the default without
    /// being keyed by it). Shares `shadowed_by`'s approximation.
    pub applies_to_default: bool,
}

/// The stored side of `auth status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredCredentialsStatus {
    /// Stored logins were read (possibly none).
    Available(Vec<StoredCredentialStatus>),
    /// No usable OS keyring backend on this host; only environment
    /// credentials apply.
    BackendUnavailable { reason: String },
}

/// The unified `auth status` view: everything sysand will authenticate
/// with, from both sources. Never contains secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStatus {
    pub stored: StoredCredentialsStatus,
    pub env: Vec<EnvCredentialEntry>,
}

/// Validate that `index_url` is an absolute HTTP(S) index location (a
/// plain URL or a `{path}`/`{path_raw}` URL template) and normalize it to
/// its credential store key form.
///
/// A plain URL normalizes via [`normalize_index_key`]. A template's key
/// rewrites only the anchorable literal prefix through `url::Url`
/// serialization; the rest stays verbatim (raw template text, not a
/// parsed URL). The result is idempotent and round-trips through
/// [`IndexLocation::parse`] / `Display`, so `auth status` prints template
/// keys in the exact form `auth logout` accepts. A template with no safe
/// anchor is rejected ([`AuthCommandError::TemplateWithoutAnchor`]).
///
/// Public so the CLI can validate the target before reading a secret.
pub fn validated_index_key(index_url: &str) -> Result<String, AuthCommandError> {
    // Before `Url::parse`: it would percent-encode the braces into a
    // mangled `%7Bpath%7D` key.
    if is_template_syntax(index_url) {
        return normalized_template_key(index_url);
    }
    // Scheme check first, so a non-HTTP(S) location gets the dedicated
    // message instead of a generic normalization error.
    let url = Url::parse(index_url)
        .map_err(|err| AuthCommandError::InvalidIndexUrl(format!("`{index_url}`: {err}")))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(AuthCommandError::NotHttpIndex {
                url: index_url.to_string(),
            });
        }
    }
    normalize_index_key(index_url).map_err(|err| match err {
        CredentialStoreError::InvalidIndexUrl(msg) => AuthCommandError::InvalidIndexUrl(msg),
        other => AuthCommandError::Store(other),
    })
}

/// The credential-key form of a URL-template target: see
/// [`validated_index_key`].
fn normalized_template_key(index_url: &str) -> Result<String, AuthCommandError> {
    let location = IndexLocation::parse(index_url).map_err(|err| match err {
        IndexLocationError::UnsupportedScheme { .. } => AuthCommandError::NotHttpIndex {
            url: index_url.to_string(),
        },
        // The index-location errors already name the URL; no re-prefixing.
        other => AuthCommandError::InvalidIndexUrl(other.to_string()),
    })?;
    let IndexLocation::Template(template) = &location else {
        // `IndexLocation::parse` classifies every brace-containing string
        // as a template, and `index_url` contains a brace.
        unreachable!("BUG: template syntax parsed as a plain URL");
    };
    let prefix = template.prefix();
    let Some((cut, anchor)) = template_anchor_root(prefix) else {
        return Err(AuthCommandError::TemplateWithoutAnchor {
            url: index_url.to_string(),
        });
    };
    // `Display` reproduces the validated template text; the prefix past
    // its last `/` contains no `/`, so reassembly keeps the anchor as the
    // key's own anchor (idempotence).
    let tail = location.to_string();
    Ok(format!(
        "{anchor}{}{}",
        &prefix[cut + 1..],
        &tail[prefix.len()..]
    ))
}

/// Progress notice emitted by [`do_auth_login`] while it works, so the
/// host can render it at the right moment (in particular, a replacement is
/// reported strictly before the store write happens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthLoginNotice {
    /// A stored credential for the same key exists and is about to be
    /// overwritten.
    ReplacingExisting { key: String },
    /// The discovery document could not be read: a network failure, or an
    /// HTTP error that survived login's authenticated retry (see
    /// [`fetch_login_endpoints`]). The credential is scoped to the
    /// URL-derived pattern only.
    DiscoveryUnreachable { error: String },
    /// Discovery resolved a templated `index_root` with no safe anchor,
    /// so no glob was derived: deriving one anyway could produce a
    /// pattern matching other hosts.
    TemplateIndexRootSkipped { template: String },
    /// A validation probe was answered with a redirect. Probes never
    /// follow redirects (the verdict would come from a different URL),
    /// so the surface counts as not tested.
    ProbeRedirected {
        surface: ProbeSurface,
        target: String,
    },
    /// A validation probe could not produce a verdict (network error,
    /// 5xx, or an unexpected status); the surface counts as not tested.
    ProbeUnreachable {
        surface: ProbeSurface,
        error: String,
    },
    /// A validation probe was answered with HTTP 429. A 429 is never a
    /// verdict: the surface counts as not tested, so rate
    /// limiting can never refuse a credential.
    ProbeRateLimited { surface: ProbeSurface },
    /// A surface rejected the credential but another accepted it, so it
    /// was stored anyway. `read_status` is the read surface's rejection
    /// status (a 404 hedges the warning: possibly no `index.json` at that
    /// URL); `basic_challenge` means the read surface answered with a
    /// `WWW-Authenticate: Basic` challenge.
    SurfaceRejected {
        surface: ProbeSurface,
        read_status: Option<u16>,
        basic_challenge: bool,
    },
}

/// Outcome of [`do_auth_login`].
///
/// An absent keyring backend is an outcome rather than an error because
/// the host still needs the derived globs: the CLI prints the exact
/// `SYSAND_CRED_*` lines to set instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthLoginOutcome {
    /// The credential was persisted.
    Stored {
        key: String,
        globs: Vec<String>,
        /// The surfaces that exercised and accepted the credential, in
        /// probe order. Empty means "stored, not validated". Hosts must
        /// scope the claim to these surfaces and never print a bare
        /// "validated".
        validated: Vec<ProbeSurface>,
    },
    /// No usable OS keyring backend on this host; nothing was persisted
    /// and the secret was discarded.
    BackendUnavailable {
        key: String,
        globs: Vec<String>,
        reason: String,
    },
}

/// Derive the escaped URL glob patterns a login covers.
///
/// The primary glob anchors on `primary_root`
/// (`globset::escape(<root ending in />)` + `**`); the resolved
/// `index_root` and `api_root` add globs only when not covered by an
/// earlier root, and a root that prefixes an earlier one subsumes it. A
/// templated `index_root` anchors on its literal prefix cut back to the
/// last `/`: under `literal_separator(true)` a mid-segment anchor would
/// not match. Both derivation and runtime matching serialize URLs via
/// `url::Url::as_str()`, so IDN/percent-encoding agree on both sides.
fn derive_credential_globs(
    primary_root: &str,
    endpoints: Option<&ResolvedEndpoints>,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> Vec<String> {
    // Roots already covered, each ending in `/`. A candidate whose string
    // prefix is an existing root is covered: that root's `**` glob
    // matches every URL under the candidate.
    let mut roots: Vec<String> = vec![primary_root.to_string()];

    let push_if_uncovered = |roots: &mut Vec<String>, candidate: &str| {
        if !roots
            .iter()
            .any(|root| candidate.starts_with(root.as_str()))
        {
            // A candidate that prefixes an existing root subsumes it.
            roots.retain(|root| !root.starts_with(candidate));
            roots.push(candidate.to_string());
        }
    };

    if let Some(endpoints) = endpoints {
        match &endpoints.index_root {
            IndexLocation::Root(url) => push_if_uncovered(&mut roots, url.as_str()),
            IndexLocation::Template(template) => match template_anchor_root(template.prefix()) {
                Some((_, anchor)) => push_if_uncovered(&mut roots, anchor.as_str()),
                None => notify(AuthLoginNotice::TemplateIndexRootSkipped {
                    template: template.to_string(),
                }),
            },
        }
        if let Some(api_root) = &endpoints.api_root {
            push_if_uncovered(&mut roots, api_root.as_str());
        }
    }

    roots
        .into_iter()
        .map(|root| format!("{}**", globset::escape(&root)))
        .collect()
}

/// Anchor a URL template's literal prefix at a safe URL boundary: the
/// prefix cut back to its last `/`, reparsed as a URL so the anchor uses
/// the same serialization runtime request URLs will. Returns the cut
/// position (byte index of that `/`) alongside the anchor, or `None` when
/// no anchor at least as deep as `scheme://authority/` exists.
fn template_anchor_root(prefix: &str) -> Option<(usize, Url)> {
    let cut = prefix.rfind('/')?;
    let candidate = &prefix[..=cut];
    let url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Some((cut, url))
}

/// Identity fields parsed from a successful `v1/whoami` response:
/// persisted on the stored record by a validating login and rendered live
/// by `auth whoami`. Never contains the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoamiIdentity {
    pub subject: CredentialSubject,
    pub token_name: Option<String>,
    pub token_prefix: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Wire shape of a `v1/whoami` 200 body (design/index-api-protocol.md,
/// Token Identity). Parsed leniently: an unparseable body keeps the 200
/// verdict (accepted) and merely loses the identity fields.
#[derive(Deserialize)]
struct WhoamiBody {
    subject: WhoamiSubject,
    token: WhoamiToken,
}

#[derive(Deserialize)]
struct WhoamiSubject {
    #[serde(rename = "type")]
    kind: String,
    name: String,
}

#[derive(Deserialize)]
struct WhoamiToken {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
    // chrono's serde impl parses the protocol's RFC 3339 timestamp.
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

/// Read and parse a `v1/whoami` `200` body into the identity fields.
/// Lenient: a body this client cannot read only loses the identity (the
/// `200` was already the acceptance verdict), so failures return `None`.
fn parse_whoami_identity(
    runtime: &tokio::runtime::Runtime,
    response: reqwest::Response,
) -> Option<WhoamiIdentity> {
    let parsed = runtime
        .block_on(response.bytes())
        .map_err(|err| err.to_string())
        .and_then(|body| {
            serde_json::from_slice::<WhoamiBody>(&body).map_err(|err| err.to_string())
        });
    match parsed {
        Ok(body) => Some(WhoamiIdentity {
            subject: CredentialSubject {
                kind: body.subject.kind,
                name: body.subject.name,
            },
            token_name: body.token.name,
            token_prefix: body.token.prefix,
            expires_at: body.token.expires_at,
        }),
        Err(err) => {
            log::debug!("whoami body was not read: {err}");
            None
        }
    }
}

/// What the validation probes concluded. A surface appears in
/// `accepted` or `rejected` only when it
/// actually exercised the credential; surfaces that were public,
/// redirected, or unreachable appear in neither (they were reported as
/// notices instead).
#[derive(Default)]
struct ProbeOutcome {
    accepted: Vec<ProbeSurface>,
    rejected: Vec<ProbeSurface>,
    /// The HTTP status of the read surface's rejection, when it rejected.
    read_status: Option<u16>,
    basic_challenge: bool,
    identity: Option<WhoamiIdentity>,
}

/// Build the dedicated probe client: same user agent as the runtime
/// client, but with redirects disabled (a redirected probe's verdict
/// would come from a different URL than the surface nominally probed, and
/// a cross-host redirect strips the Authorization header, misreading
/// "rejected") and a timeout so a hung probe cannot hang `auth login`.
fn probe_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(crate::resolve::net_utils::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

/// Send one probe GET, optionally with a forced bearer credential.
/// Returns the response unread (probes never consume bodies; the whoami
/// body is read separately on a 200).
fn probe_get(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    url: &Url,
    bearer: Option<&str>,
) -> Result<reqwest::Response, String> {
    runtime.block_on(async {
        let mut request = client.get(url.clone());
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        request.send().await.map_err(|err| err.to_string())
    })
}

/// The redirect target for a 3xx probe response, for the "not tested"
/// warning. Kept verbatim (possibly relative); a missing `Location`
/// header is named as such rather than printed empty.
fn redirect_target(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| "(no Location header)".to_string())
}

/// Whether any `WWW-Authenticate` challenge on the response offers the
/// `Basic` scheme (case-insensitive, RFC 7235). Splits on top-level
/// commas only and checks each challenge's leading scheme token, because
/// both a substring check and a naive comma split false-positive on
/// quoted realm values. (A quoted `\"` is not handled; this only tunes a
/// hint message.)
fn offers_basic_challenge(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(header_offers_basic)
}

fn header_offers_basic(value: &str) -> bool {
    // Split into challenges on commas outside double quotes.
    let mut in_quotes = false;
    value
        .split(|c: char| {
            if c == '"' {
                in_quotes = !in_quotes;
            }
            c == ',' && !in_quotes
        })
        .any(|challenge| {
            let token = challenge
                .trim_start()
                .split([' ', '\t'])
                .next()
                .unwrap_or("");
            token.eq_ignore_ascii_case("basic")
        })
}

/// Probe the read surface (`index_root/index.json`): an unauthenticated
/// baseline, then a forced-bearer retry only when the baseline was a 4xx.
/// The surface exercises the credential only in that case (a public
/// surface returns 2xx without the credential ever being sent, proving
/// nothing).
fn probe_read_surface(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    index_json_url: &Url,
    secret: &str,
    outcome: &mut ProbeOutcome,
    notify: &mut impl FnMut(AuthLoginNotice),
) {
    let surface = ProbeSurface::Read;
    let baseline = match probe_get(runtime, client, index_json_url, None) {
        Ok(response) => response,
        Err(error) => {
            notify(AuthLoginNotice::ProbeUnreachable { surface, error });
            return;
        }
    };
    let status = baseline.status();
    if status.is_redirection() {
        notify(AuthLoginNotice::ProbeRedirected {
            surface,
            target: redirect_target(&baseline),
        });
        return;
    }
    if status.is_success() {
        // Public read surface: the credential was never sent, so the
        // surface is not tested (no notice; this is the normal public
        // case, not a failure).
        return;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Never force-retry a rate-limited surface (429 is never a
        // verdict).
        notify(AuthLoginNotice::ProbeRateLimited { surface });
        return;
    }
    if !status.is_client_error() {
        notify(AuthLoginNotice::ProbeUnreachable {
            surface,
            error: format!("unexpected status {status}"),
        });
        return;
    }
    let basic_challenge = offers_basic_challenge(baseline.headers());
    let forced = match probe_get(runtime, client, index_json_url, Some(secret)) {
        Ok(response) => response,
        Err(error) => {
            notify(AuthLoginNotice::ProbeUnreachable { surface, error });
            return;
        }
    };
    let forced_status = forced.status();
    if forced_status.is_redirection() {
        notify(AuthLoginNotice::ProbeRedirected {
            surface,
            target: redirect_target(&forced),
        });
    } else if forced_status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // 429 is never a verdict: counting it as rejected
        // would let throttling false-refuse a valid token.
        notify(AuthLoginNotice::ProbeRateLimited { surface });
    } else if forced_status.is_success() {
        outcome.accepted.push(surface);
    } else if forced_status.is_client_error() {
        outcome.rejected.push(surface);
        outcome.read_status = Some(forced_status.as_u16());
        outcome.basic_challenge = basic_challenge || offers_basic_challenge(forced.headers());
    } else {
        notify(AuthLoginNotice::ProbeUnreachable {
            surface,
            error: format!("unexpected status {forced_status}"),
        });
    }
}

/// Probe the API surface (`api_root/v1/whoami`), forced-only: the
/// endpoint is always authenticated, so its unauthenticated baseline is a
/// known 401 and only the forced request is sent. 200 accepts (and its
/// body carries the identity to persist), 401 rejects, anything else is
/// not tested.
fn probe_api_surface(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    api_root: &Url,
    secret: &str,
    outcome: &mut ProbeOutcome,
    notify: &mut impl FnMut(AuthLoginNotice),
) {
    let surface = ProbeSurface::Api;
    // `api_root` always ends in `/` (discovery normalizes it), so the
    // join appends rather than replacing the last segment.
    let whoami_url = match api_root.join("v1/whoami") {
        Ok(url) => url,
        Err(err) => {
            notify(AuthLoginNotice::ProbeUnreachable {
                surface,
                error: format!("could not build the whoami URL: {err}"),
            });
            return;
        }
    };
    let response = match probe_get(runtime, client, &whoami_url, Some(secret)) {
        Ok(response) => response,
        Err(error) => {
            notify(AuthLoginNotice::ProbeUnreachable { surface, error });
            return;
        }
    };
    let status = response.status();
    if status.is_redirection() {
        notify(AuthLoginNotice::ProbeRedirected {
            surface,
            target: redirect_target(&response),
        });
    } else if status == reqwest::StatusCode::OK {
        outcome.accepted.push(surface);
        outcome.identity = parse_whoami_identity(runtime, response);
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        outcome.rejected.push(surface);
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // 429 is never a verdict; the protocol explicitly
        // allows rate limiting on `v1/whoami`.
        notify(AuthLoginNotice::ProbeRateLimited { surface });
    } else {
        notify(AuthLoginNotice::ProbeUnreachable {
            surface,
            error: format!("unexpected status {status}"),
        });
    }
}

/// Fetch discovery for a login: an unauthenticated baseline, then one
/// forced-bearer retry on any 4xx except 429. The forced retry distinguishes a hidden discovery
/// document from an absent one: a fully private index answers 401 (or
/// GitLab-style 404) unauthenticated either way, and only the
/// credentialed answer settles it.
///
/// Discovery is about topology, not validation; its outcome never refuses
/// a login. `None` means "no usable document" and the caller falls back
/// to the URL-derived glob after a
/// [`AuthLoginNotice::DiscoveryUnreachable`] notice.
fn fetch_login_endpoints(
    client: &reqwest_middleware::ClientWithMiddleware,
    location: &IndexLocation,
    secret: &str,
    runtime: &tokio::runtime::Runtime,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> Option<ResolvedEndpoints> {
    // The baseline uses the regular middleware client; the strict variant
    // surfaces an absent document as its 404 status so the retry decision
    // below can see it.
    match runtime.block_on(fetch_index_config_strict(
        client,
        &Unauthenticated {},
        location,
    )) {
        Ok(endpoints) => Some(endpoints),
        Err(DiscoveryError::Fetch(HttpFetchError::BadHttpStatus { status, .. }))
            if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS =>
        {
            forced_discovery_fetch(location, secret, runtime, notify)
        }
        // Network failures, 5xx, a 429 baseline, and a present-but-invalid
        // document are not "possibly hidden from me" signals: no secret
        // is sent.
        Err(err) => {
            notify(AuthLoginNotice::DiscoveryUnreachable {
                error: err.to_string(),
            });
            None
        }
    }
}

/// The forced leg of [`fetch_login_endpoints`]: one GET of the discovery
/// document with a forced bearer, through the no-redirect probe client.
/// 200 with a valid document is used like a public discovery success; 404
/// is the authoritative "no document" answer and reconstructs the flat
/// topology (no notice). Everything else falls back to `None` with a
/// notice; the validation probes still deliver the credential verdict.
fn forced_discovery_fetch(
    location: &IndexLocation,
    secret: &str,
    runtime: &tokio::runtime::Runtime,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> Option<ResolvedEndpoints> {
    let config_url = location.resolve([INDEX_CONFIG_PATH]);
    let mut unreachable = |error: String| {
        notify(AuthLoginNotice::DiscoveryUnreachable { error });
        None
    };
    let client = match probe_client() {
        Ok(client) => client,
        Err(err) => {
            return unreachable(format!("could not build the probe HTTP client: {err}"));
        }
    };
    let response = match probe_get(runtime, &client, &config_url, Some(secret)) {
        Ok(response) => response,
        Err(error) => {
            return unreachable(format!("HTTP request to `{config_url}` failed: {error}"));
        }
    };
    let status = response.status();
    if status == reqwest::StatusCode::OK {
        let raw = runtime
            .block_on(response.bytes())
            .map_err(|err| format!("failed to read HTTP response body from `{config_url}`: {err}"))
            .and_then(|body| {
                serde_json::from_slice::<IndexConfigRaw>(&body)
                    .map_err(|err| format!("failed to parse JSON from `{config_url}`: {err}"))
            });
        match raw {
            Ok(raw) => match resolve_index_config(raw, &config_url, location.clone()) {
                Ok(endpoints) => Some(endpoints),
                Err(err) => unreachable(err.to_string()),
            },
            Err(error) => unreachable(error),
        }
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Some(ResolvedEndpoints::flat(location.clone()))
    } else if status.is_redirection() {
        // The probe client never follows redirects, so a redirect-fronted
        // discovery document gets this notice instead of a forced answer.
        unreachable(format!(
            "the authenticated retry for `{config_url}` was redirected to `{}`",
            redirect_target(&response)
        ))
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        unreachable(format!(
            "the authenticated retry for `{config_url}` was rate limited (HTTP 429)"
        ))
    } else {
        unreachable(format!(
            "the authenticated retry for `{config_url}` returned status {status}"
        ))
    }
}

/// Run the validation probes.
/// When discovery was unreachable the read surface falls back to the
/// URL-derived `index.json`; the API surface is probed only when
/// discovery advertised `api_root`, so a static index is not
/// phantom-probed for an API it does not have.
fn run_validation_probes(
    endpoints: Option<&ResolvedEndpoints>,
    location: &IndexLocation,
    secret: &str,
    runtime: &tokio::runtime::Runtime,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> ProbeOutcome {
    let mut outcome = ProbeOutcome::default();
    let client = match probe_client() {
        Ok(client) => client,
        Err(err) => {
            notify(AuthLoginNotice::ProbeUnreachable {
                surface: ProbeSurface::Read,
                error: format!("could not build the probe HTTP client: {err}"),
            });
            return outcome;
        }
    };

    // For a templated `index_root` this expands the placeholder exactly
    // like runtime reads of the top-level `index.json` do, so the probe
    // hits the real read surface.
    let index_json_url = match endpoints {
        Some(endpoints) => endpoints.index_url(),
        None => location.resolve(["index.json"]),
    };
    probe_read_surface(
        runtime,
        &client,
        &index_json_url,
        secret,
        &mut outcome,
        notify,
    );

    // `api_root` is set only when discovery advertises it, so only an index
    // with an API gets probed.
    if let Some(endpoints) = endpoints
        && let Some(api_root) = &endpoints.api_root
    {
        probe_api_surface(runtime, &client, api_root, secret, &mut outcome, notify);
    }

    outcome
}

/// Best-effort discovery used only to scope the `SYSAND_CRED_*` guidance
/// globs when the keyring backend is absent. Strictly unauthenticated
/// (the secret is discarded on this path); a discovery failure degrades
/// to the URL-derived glob with a warning.
fn guidance_globs(
    client: &reqwest_middleware::ClientWithMiddleware,
    location: &IndexLocation,
    primary_root: &str,
    runtime: &tokio::runtime::Runtime,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> Vec<String> {
    let endpoints =
        match runtime.block_on(fetch_index_config(client, &Unauthenticated {}, location)) {
            Ok(endpoints) => Some(endpoints),
            Err(err) => {
                notify(AuthLoginNotice::DiscoveryUnreachable {
                    error: err.to_string(),
                });
                None
            }
        };
    derive_credential_globs(primary_root, endpoints.as_ref(), notify)
}

/// Store a bearer credential for `index_url`. The secret arrives as a
/// parameter: a library call never prompts.
///
/// Discovery is fetched best-effort for glob scoping
/// ([`fetch_login_endpoints`]); with no usable document the credential
/// falls back to the URL-derived glob. Validation always runs, between
/// glob derivation and the store write, with the refusal rule:
/// stored if any exercised surface accepted, refused
/// ([`AuthCommandError::ValidationRejected`]) when surfaces only
/// rejected, stored "not validated" when nothing exercised the credential
/// (how an offline index degrades). Replacement is announced via
/// [`AuthLoginNotice::ReplacingExisting`] before the write, and only once
/// validation has decided the write will happen. An absent keyring
/// backend returns [`AuthLoginOutcome::BackendUnavailable`] before any
/// probe runs: no network is spent on a credential that cannot be stored.
pub fn do_auth_login<B: BlobBackend>(
    store: &mut LockedBlobStore<B>,
    index_url: &str,
    secret: String,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &tokio::runtime::Runtime,
    mut notify: impl FnMut(AuthLoginNotice),
) -> Result<AuthLoginOutcome, AuthCommandError> {
    let key = validated_index_key(index_url)?;
    let location = IndexLocation::parse(&key)
        .map_err(|err| AuthCommandError::InvalidIndexUrl(format!("`{key}`: {err}")))?;
    // Key validation already rejected unanchorable templates; this
    // re-check just avoids a panic path.
    let primary_root = match &location {
        IndexLocation::Root(_) => key.clone(),
        IndexLocation::Template(template) => match template_anchor_root(template.prefix()) {
            Some((_, anchor)) => anchor.as_str().to_string(),
            None => {
                return Err(AuthCommandError::TemplateWithoutAnchor { url: key });
            }
        },
    };

    // Read the store before any network: an absent keyring backend must
    // be detected before the secret could be spent on a credentialed
    // request. Also detects an existing login for the replacement notice;
    // `list` and `upsert` are separately locked, so that part is
    // best-effort under cross-process races.
    let records = match store.list() {
        Ok(records) => records,
        Err(CredentialStoreError::BackendAbsent { source }) => {
            let globs = guidance_globs(client, &location, &primary_root, runtime, &mut notify);
            return Ok(AuthLoginOutcome::BackendUnavailable {
                key,
                globs,
                reason: source.to_string(),
            });
        }
        Err(err) => return Err(err.into()),
    };
    let replacing = records.iter().any(|record| record.key == key);

    let endpoints = fetch_login_endpoints(client, &location, &secret, runtime, &mut notify);
    let globs = derive_credential_globs(&primary_root, endpoints.as_ref(), &mut notify);

    let outcome =
        run_validation_probes(endpoints.as_ref(), &location, &secret, runtime, &mut notify);
    // Refuse only when an exercised surface rejected and none accepted.
    if outcome.accepted.is_empty() && !outcome.rejected.is_empty() {
        return Err(AuthCommandError::ValidationRejected {
            index: key,
            rejected: outcome.rejected,
            read_status: outcome.read_status,
            basic_challenge: outcome.basic_challenge,
        });
    }
    // Stored anyway: warn about each surface that rejected.
    for surface in &outcome.rejected {
        notify(AuthLoginNotice::SurfaceRejected {
            surface: *surface,
            read_status: if *surface == ProbeSurface::Read {
                outcome.read_status
            } else {
                None
            },
            basic_challenge: outcome.basic_challenge && *surface == ProbeSurface::Read,
        });
    }
    let validated = outcome.accepted;
    let identity = outcome.identity;

    if replacing {
        notify(AuthLoginNotice::ReplacingExisting { key: key.clone() });
    }

    let (subject, token_name, token_prefix, expires_at) = match identity {
        Some(identity) => (
            Some(identity.subject),
            identity.token_name,
            identity.token_prefix,
            identity.expires_at,
        ),
        None => (None, None, None, None),
    };
    let record = CredentialRecord {
        key: key.clone(),
        globs: globs.clone(),
        scheme: CredentialScheme::Bearer,
        secret,
        expires_at,
        subject,
        token_name,
        token_prefix,
        // Persist the same scoped claim the host prints ("validated
        // (read)" and so on), so `auth status` can show it later.
        validated: validated
            .iter()
            .map(std::string::ToString::to_string)
            .collect(),
        extra: serde_json::Map::new(),
    };
    match store.upsert(record) {
        Ok(()) => Ok(AuthLoginOutcome::Stored {
            key,
            globs,
            validated,
        }),
        Err(CredentialStoreError::BackendAbsent { source }) => {
            Ok(AuthLoginOutcome::BackendUnavailable {
                key,
                globs,
                reason: source.to_string(),
            })
        }
        Err(err) => Err(err.into()),
    }
}

/// Remove the stored credential for `index_url`.
///
/// Returns the normalized index key the record was stored under. Removing
/// a login that does not exist is an error
/// ([`AuthCommandError::NoStoredCredential`]).
pub fn do_auth_logout<B: BlobBackend>(
    store: &mut LockedBlobStore<B>,
    index_url: &str,
) -> Result<String, AuthCommandError> {
    let key = validated_index_key(index_url)?;
    if store.remove(&key)? {
        Ok(key)
    } else {
        Err(AuthCommandError::NoStoredCredential { index: key })
    }
}

/// Where the credential `auth whoami` selected came from. Selection
/// mirrors publish's source precedence: environment credentials before
/// stored credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhoamiCredentialSource {
    /// A `SYSAND_CRED_*` environment bearer; `label` is the
    /// `SYSAND_CRED_<LABEL>` stem when known.
    Env { label: Option<String> },
    /// A stored credential (`sysand auth login`) for the given index key.
    Stored { key: String },
}

/// Verdict of the single `v1/whoami` request `auth whoami` sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhoamiVerdict {
    /// HTTP 200: the index accepted the credential. `identity` is `None`
    /// when the response body could not be parsed (the 200 remains the
    /// verdict, only the details are lost).
    Identified { identity: Option<WhoamiIdentity> },
    /// HTTP 401: the index rejected the credential.
    Rejected,
    /// No verdict: a redirect, rate limiting, an unexpected status, or a
    /// network error, described by `detail`.
    Unreachable { detail: String },
}

/// What [`do_auth_whoami`] found: the normalized index key, the exact URL
/// asked, which credential was sent (the host must name it: an env
/// credential shadows a stored credential, so "re-login" would be the wrong
/// remediation for a rejected env credential), and the verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthWhoamiOutcome {
    pub key: String,
    pub whoami_url: Url,
    pub source: WhoamiCredentialSource,
    pub verdict: WhoamiVerdict,
}

/// Select the credential `auth whoami` sends, mirroring the runtime and
/// publish precedence: env
/// over stored, and within a source exactly one credential may match
/// (same-token candidates collapse, [`select_bearer`]). The stored map is
/// built by the same [`stored_bearer_map_from_records`] the runtime uses,
/// so one bad glob pattern cannot hide the other credentials.
fn select_whoami_credential(
    env_bearers: &GlobMap<EnvBearerAuth>,
    records: &[CredentialRecord],
    whoami_url: &Url,
    index_key: &str,
) -> Result<(WhoamiCredentialSource, String), AuthCommandError> {
    match select_bearer(env_bearers, whoami_url.as_str(), |entry| entry.auth.token()) {
        BearerSelection::Unique(entry) => {
            return Ok((
                WhoamiCredentialSource::Env {
                    label: entry.label.clone(),
                },
                entry.auth.token().to_string(),
            ));
        }
        BearerSelection::Ambiguous { candidates, .. } => {
            return Err(AuthCommandError::AmbiguousWhoamiCredential {
                url: whoami_url.as_str().to_string(),
                source_name: "`SYSAND_CRED_*` environment variables",
                candidates,
            });
        }
        BearerSelection::None => {}
    }

    let stored = stored_bearer_map_from_records(records);
    match select_bearer(&stored, whoami_url.as_str(), |entry| entry.auth().token()) {
        BearerSelection::Unique(entry) => Ok((
            WhoamiCredentialSource::Stored {
                key: entry.key().to_string(),
            },
            entry.auth().token().to_string(),
        )),
        BearerSelection::Ambiguous { candidates, .. } => {
            Err(AuthCommandError::AmbiguousWhoamiCredential {
                url: whoami_url.as_str().to_string(),
                source_name: "stored credentials",
                candidates,
            })
        }
        BearerSelection::None => Err(AuthCommandError::NoWhoamiCredential {
            url: whoami_url.as_str().to_string(),
            index: index_key.to_string(),
        }),
    }
}

/// Query-only live identity check: resolve the index API, select the
/// credential the runtime would use
/// ([`select_whoami_credential`]), and send one forced-bearer GET to
/// `api_root/v1/whoami` with the no-redirect probe client. Never prompts,
/// never prints, never touches a credential store: taking a `records`
/// snapshot lets the host share one store read between this selection and
/// its discovery policy, keeping whoami at one keychain touch. Cached
/// identity fields on a stored record are deliberately not refreshed.
///
/// `discovery_auth` is the discovery-fetch policy: unlike login, whoami
/// runs when a credential exists and a private index may gate its
/// discovery document, so the CLI passes its regular read policy.
pub fn do_auth_whoami<P: HTTPAuthentication>(
    records: &[CredentialRecord],
    index_url: &str,
    env_bearers: &GlobMap<EnvBearerAuth>,
    discovery_auth: &P,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &tokio::runtime::Runtime,
) -> Result<AuthWhoamiOutcome, AuthCommandError> {
    let key = validated_index_key(index_url)?;
    let location = IndexLocation::parse(&key)
        .map_err(|err| AuthCommandError::InvalidIndexUrl(format!("`{key}`: {err}")))?;

    let endpoints = runtime
        .block_on(fetch_index_config(client, discovery_auth, &location))
        .map_err(|err| AuthCommandError::WhoamiDiscoveryFailed {
            index: key.clone(),
            error: err.to_string(),
        })?;
    // No advertised `api_root` means the index has no API to ask.
    let Some(api_root) = &endpoints.api_root else {
        return Err(AuthCommandError::NoAdvertisedApi { index: key });
    };
    // `api_root` always ends in `/` (discovery normalizes it), so the
    // join appends rather than replacing the last segment.
    let whoami_url =
        api_root
            .join("v1/whoami")
            .map_err(|err| AuthCommandError::WhoamiDiscoveryFailed {
                index: key.clone(),
                error: format!("could not build the whoami URL: {err}"),
            })?;

    let (source, token) = select_whoami_credential(env_bearers, records, &whoami_url, &key)?;

    let verdict = match probe_client() {
        Err(err) => WhoamiVerdict::Unreachable {
            detail: format!("could not build the probe HTTP client: {err}"),
        },
        Ok(probe) => match probe_get(runtime, &probe, &whoami_url, Some(&token)) {
            Err(error) => WhoamiVerdict::Unreachable { detail: error },
            Ok(response) => {
                let status = response.status();
                if status == reqwest::StatusCode::OK {
                    let identity = parse_whoami_identity(runtime, response);
                    WhoamiVerdict::Identified { identity }
                } else if status == reqwest::StatusCode::UNAUTHORIZED {
                    WhoamiVerdict::Rejected
                } else if status.is_redirection() {
                    WhoamiVerdict::Unreachable {
                        detail: format!(
                            "redirected to `{}`; whoami does not follow redirects",
                            redirect_target(&response)
                        ),
                    }
                } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    WhoamiVerdict::Unreachable {
                        detail: "rate limited (HTTP 429)".to_string(),
                    }
                } else {
                    WhoamiVerdict::Unreachable {
                        detail: format!("unexpected status {status}"),
                    }
                }
            }
        },
    };
    Ok(AuthWhoamiOutcome {
        key,
        whoami_url,
        source,
        verdict,
    })
}

/// The URL-glob match target of a validated index key: the key itself for
/// a plain URL (it is normalized and ends in `/`), the literal-prefix
/// anchor for a template key. `None` when the key does not parse or the
/// template has no safe anchor.
fn index_key_glob_root(key: &str) -> Option<String> {
    match IndexLocation::parse(key).ok()? {
        IndexLocation::Root(url) => Some(url.as_str().to_string()),
        IndexLocation::Template(template) => {
            template_anchor_root(template.prefix()).map(|(_, anchor)| anchor.as_str().to_string())
        }
    }
}

/// Whether `pattern` matches `target`, compiled the same way runtime
/// matching does (`literal_separator(true)`). Lenient: an invalid pattern
/// matches nothing instead of erroring, because `auth status` must stay
/// usable to diagnose exactly such patterns.
fn lenient_glob_matches(pattern: &str, target: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher().is_match(target))
        .unwrap_or(false)
}

/// Assemble the `auth status` view from stored records and environment
/// entries, against the given clock. Exposed for deterministic tests;
/// [`do_auth_status`] is the store-reading entry point.
///
/// `default_key` is the normalized key of the default index, when the
/// caller could resolve one; entries that apply to it are marked
/// (`applies_to_default`), answering "which credential will bare commands
/// use". Purely diagnostic: `None` simply marks nothing.
pub fn assemble_auth_status(
    records: Vec<CredentialRecord>,
    env: Vec<EnvCredentialEntry>,
    now: DateTime<Utc>,
    default_key: Option<&str>,
) -> AuthStatus {
    let default_glob_root = default_key.and_then(index_key_glob_root);
    let default_applies = |globs: &[String]| {
        default_glob_root
            .as_deref()
            .is_some_and(|root| globs.iter().any(|glob| lenient_glob_matches(glob, root)))
    };
    let mut env = env;
    for entry in &mut env {
        entry.applies_to_default = default_applies(std::slice::from_ref(&entry.pattern));
    }
    // Compiled like runtime matching (`literal_separator(true)`); an
    // invalid pattern cannot shadow anything and is skipped.
    let env_matchers: Vec<(&str, globset::GlobMatcher)> = env
        .iter()
        .filter_map(|entry| {
            GlobBuilder::new(&entry.pattern)
                .literal_separator(true)
                .build()
                .ok()
                .map(|glob| (entry.label.as_str(), glob.compile_matcher()))
        })
        .collect();

    let stored = records
        .into_iter()
        .map(|record| {
            let shadowed_by = env_matchers
                .iter()
                .filter(|(_, matcher)| matcher.is_match(&record.key))
                .map(|(label, _)| (*label).to_string())
                .collect();
            StoredCredentialStatus {
                expired: record.expires_at.is_some_and(|expiry| expiry < now),
                expires_in_days: record.expires_at.map(|expiry| (expiry - now).num_days()),
                applies_to_default: default_key == Some(record.key.as_str())
                    || default_applies(&record.globs),
                key: record.key,
                globs: record.globs,
                expires_at: record.expires_at,
                subject: record.subject,
                token_prefix: record.token_prefix,
                validated: record.validated,
                shadowed_by,
            }
        })
        .collect();

    AuthStatus {
        stored: StoredCredentialsStatus::Available(stored),
        env,
    }
}

/// Read the stored credentials and assemble the unified `auth status` view.
///
/// An absent keyring backend degrades to the env-only view
/// ([`StoredCredentialsStatus::BackendUnavailable`]); a present but locked
/// or denied backend is a hard error the caller must surface.
pub fn do_auth_status<B: BlobBackend>(
    store: &LockedBlobStore<B>,
    env: Vec<EnvCredentialEntry>,
    default_key: Option<&str>,
) -> Result<AuthStatus, AuthCommandError> {
    match store.list() {
        Ok(records) => Ok(assemble_auth_status(records, env, Utc::now(), default_key)),
        Err(CredentialStoreError::BackendAbsent { source }) => Ok(AuthStatus {
            stored: StoredCredentialsStatus::BackendUnavailable {
                reason: source.to_string(),
            },
            env,
        }),
        Err(err) => Err(err.into()),
    }
}

// Private tests

#[cfg(test)]
#[path = "./auth_tests.rs"]
mod tests;
