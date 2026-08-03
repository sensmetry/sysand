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
        ValidatedSurface,
        keyring_store::{BlobBackend, LockedBlobStore},
    },
    env::discovery::{
        DiscoveryError, INDEX_CONFIG_PATH, IndexConfigRaw, ResolvedEndpoints, fetch_index_config,
        fetch_index_config_strict, resolve_index_config,
    },
    env::index::HttpFetchError,
    index_location::{IndexLocation, IndexLocationError, IndexUrlTemplate},
};

/// Errors from the `sysand auth` commands.
#[derive(Debug, Error)]
pub enum AuthCommandError {
    /// `logout` targeted an index with no stored credential.
    #[error("no stored credential for `{index}`")]
    NoStoredCredential { index: String },
    /// The target is not an HTTP(S) index (for example a `file://` URL).
    #[error(
        "`{url}`: not an HTTP(S) index; nothing to authenticate to\n\
         (use an https:// index URL)"
    )]
    NotHttpIndex { url: String },
    /// The target could not be parsed or normalized as an index URL.
    #[error("invalid index URL for credential storage: {0}")]
    InvalidIndexUrl(String),
    /// Every exercised surface rejected the credential and none accepted
    /// it, so nothing was stored.
    #[error("{}", validation_rejected_message(.index, .rejected, .challenge_schemes))]
    ValidationRejected {
        /// The normalized index key the login targeted.
        index: String,
        /// The surfaces that exercised and rejected the credential, each
        /// with the HTTP status it answered. A read-surface 404 hedges
        /// the message: it can mean a rejected token, but also a URL with
        /// no index at all.
        rejected: Vec<(ProbeSurface, u16)>,
        /// The scheme tokens the read surface offered in its
        /// `WWW-Authenticate` challenges, verbatim in first-seen order
        /// (deduplicated case-insensitively). `Basic` routes the message
        /// to `SYSAND_CRED_*` basic credentials; schemes other than
        /// `Basic`/`Bearer` are named as unsupported.
        challenge_schemes: Vec<String>,
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
    /// single identity question can be asked. `candidates` names every
    /// matching credential (env variable names, or backticked stored
    /// keys) so the error is actionable.
    #[error(
        "multiple credentials from {source_name} match `{url}`:\n\
         {};\n\
         refine the patterns so exactly one matches",
        candidates.join(", ")
    )]
    AmbiguousWhoamiCredential {
        url: String,
        // Not `source`: thiserror reserves that name for error chaining.
        source_name: &'static str,
        candidates: Vec<String>,
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
    rejected: &[(ProbeSurface, u16)],
    challenge_schemes: &[String],
) -> String {
    let mut message = match rejected {
        // A read-surface 404 is hedged: it can also mean there is simply
        // no index at this URL, so the token must not be blamed outright.
        [(surface, status)] => {
            let endpoint = match surface {
                ProbeSurface::Read => "index.json",
                ProbeSurface::Api => "v1/whoami",
            };
            let hedge = if *surface == ProbeSurface::Read && *status == 404 {
                ", or no index exists at this URL"
            } else {
                ""
            };
            format!(
                "the index rejected the token for `{index}`\n\
                 (`{endpoint}` answered HTTP {status}){hedge}; nothing was stored"
            )
        }
        _ => {
            let surfaces: Vec<String> = rejected
                .iter()
                .map(|(surface, status)| match surface {
                    ProbeSurface::Read => {
                        format!("the index read surface (`index.json`, HTTP {status})")
                    }
                    ProbeSurface::Api => format!("the index API (`v1/whoami`, HTTP {status})"),
                })
                .collect();
            format!(
                "credential for `{index}` was rejected by {} and accepted by no surface;\n\
                 nothing was stored",
                surfaces.join(" and ")
            )
        }
    };
    if schemes_include_basic(challenge_schemes) {
        // Keep this basic-auth routing hint consistent with the
        // stored-anyway variant in sysand/src/commands/auth.rs
        // (render_login_notice).
        message.push_str(
            "\nthis index uses username/password (HTTP basic) authentication; configure\n\
             `SYSAND_CRED_<X>_BASIC_USER` / `SYSAND_CRED_<X>_BASIC_PASS` environment\n\
             variables instead",
        );
    }
    if let Some(followup) = unsupported_schemes_followup(challenge_schemes) {
        message.push('\n');
        message.push_str(&followup);
    }
    message
}

/// Whether the collected challenge schemes include HTTP `Basic`
/// (case-insensitive): the index wants username/password, not a bearer
/// token, and messages route the user to `SYSAND_CRED_*` basic
/// credentials.
pub fn schemes_include_basic(schemes: &[String]) -> bool {
    schemes.iter().any(|s| s.eq_ignore_ascii_case("basic"))
}

/// The follow-up line naming, verbatim, the offered challenge schemes
/// sysand does not support (anything other than `Basic` and `Bearer`),
/// or `None` when every offered scheme is supported. Shared by the
/// refusal message and the stored-anyway warning so both name the same
/// schemes.
pub fn unsupported_schemes_followup(schemes: &[String]) -> Option<String> {
    let unsupported: Vec<String> = schemes
        .iter()
        .filter(|s| !s.eq_ignore_ascii_case("basic") && !s.eq_ignore_ascii_case("bearer"))
        .map(|s| format!("`{s}`"))
        .collect();
    if unsupported.is_empty() {
        return None;
    }
    let noun = if unsupported.len() == 1 {
        "scheme"
    } else {
        "schemes"
    };
    Some(format!(
        "the index requests the authentication {noun} {} in its challenge,\n\
         which sysand does not support",
        unsupported.join(", ")
    ))
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

/// The persisted form of a probe surface, for the record's `validated`
/// list.
impl From<ProbeSurface> for ValidatedSurface {
    fn from(surface: ProbeSurface) -> Self {
        match surface {
            ProbeSurface::Read => ValidatedSurface::Read,
            ProbeSurface::Api => ValidatedSurface::Api,
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
    pub validated: Vec<ValidatedSurface>,
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

/// A validated, normalized index credential key. Constructed only
/// through [`IndexKey::validate`], so holding one proves the target was
/// already validated: the `do_auth_*` commands take it instead of a raw
/// string and never re-validate.
#[derive(Debug, Clone)]
pub struct IndexKey {
    /// The normalized key, exactly as stored and printed.
    key: String,
    /// The key parsed as an index location (plain root or template), so
    /// commands need not reparse the key.
    location: IndexLocation,
    /// The URL root credential globs anchor on: the query-stripped URL
    /// for a plain key, the literal-prefix anchor for a template key.
    glob_root: String,
}

impl IndexKey {
    /// Validate that `index_url` is an absolute HTTP(S) index location (a
    /// plain URL or a `{path}`/`{path_raw}` URL template) and normalize it
    /// to its credential store key form.
    ///
    /// All index-location invariants and normalization live in
    /// [`IndexLocation::parse`]; the key is its `Display` text. Both forms
    /// are idempotent and round-trip through parse / `Display`, so
    /// `auth status` prints keys in the exact form `auth logout` accepts.
    ///
    /// This is the only validation path; the CLI calls it once, before
    /// reading a secret.
    pub fn validate(index_url: &str) -> Result<Self, AuthCommandError> {
        let location = IndexLocation::parse(index_url).map_err(|err| match err {
            IndexLocationError::UnsupportedScheme { .. } => AuthCommandError::NotHttpIndex {
                url: index_url.to_string(),
            },
            // The index-location errors already name the URL; no
            // re-prefixing.
            other => AuthCommandError::InvalidIndexUrl(other.to_string()),
        })?;
        let glob_root = location_glob_root(&location);
        Ok(Self {
            key: location.to_string(),
            location,
            glob_root,
        })
    }

    /// The normalized key string, in the exact form credentials are
    /// stored and printed under.
    pub fn as_str(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Display for IndexKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
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
    /// was stored anyway. `status` is the HTTP status the surface
    /// answered (a read-surface 404 hedges the warning: possibly no
    /// `index.json` at that URL); `challenge_schemes` carries the
    /// scheme tokens the read surface offered in `WWW-Authenticate`
    /// (always empty for the API surface).
    SurfaceRejected {
        surface: ProbeSurface,
        status: u16,
        challenge_schemes: Vec<String>,
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
) -> Vec<String> {
    // Roots already covered, each ending in `/`. A candidate whose string
    // prefix is an existing root is covered: that root's `**` glob
    // matches every URL under the candidate.
    let mut roots: Vec<String> = vec![primary_root.to_string()];

    let push_if_uncovered = |roots: &mut Vec<String>, candidate: String| {
        if !roots
            .iter()
            .any(|root| candidate.starts_with(root.as_str()))
        {
            // A candidate that prefixes an existing root subsumes it.
            roots.retain(|root| !root.starts_with(&candidate));
            roots.push(candidate);
        }
    };

    if let Some(endpoints) = endpoints {
        push_if_uncovered(&mut roots, location_glob_root(&endpoints.index_root));
        if let Some(api_root) = &endpoints.api_root {
            push_if_uncovered(&mut roots, root_anchor(api_root).into());
        }
    }

    roots
        .into_iter()
        .map(|root| format!("{}**", globset::escape(&root)))
        .collect()
}

/// Anchor a plain root URL for glob derivation: the URL without its
/// query. `resolve` appends path segments before the query, so request
/// URLs continue the path text, not the query text; a query in the
/// anchor would make the glob match nothing.
fn root_anchor(url: &Url) -> Url {
    let mut anchor = url.clone();
    anchor.set_query(None);
    anchor
}

/// Anchor a URL template's literal prefix at a safe URL boundary: the
/// prefix cut back to its last `/`, reparsed as a URL so the anchor uses
/// the same serialization runtime request URLs will. Infallible:
/// [`IndexLocation::parse`] normalized the prefix to absolute HTTP(S) URL
/// text with an explicit path `/`, so an anchor at least as deep as
/// `scheme://authority/` always exists.
fn template_anchor_root(template: &IndexUrlTemplate) -> Url {
    let prefix = template.prefix();
    let cut = prefix
        .rfind('/')
        .expect("BUG: a normalized template prefix has an explicit path `/`");
    Url::parse(&prefix[..=cut])
        .expect("BUG: a normalized template prefix cut at a `/` parses as a URL")
}

/// The URL root credential globs anchor on for `location`: the
/// query-stripped URL for a plain root, the literal-prefix anchor for a
/// template.
fn location_glob_root(location: &IndexLocation) -> String {
    match location {
        IndexLocation::Root(url) => root_anchor(url).into(),
        IndexLocation::Template(template) => template_anchor_root(template).into(),
    }
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
    let body = match runtime.block_on(response.bytes()) {
        Ok(body) => body,
        Err(err) => {
            log::debug!(
                "whoami body was not read: {}",
                crate::utils::format_err(err)
            );
            return None;
        }
    };
    match serde_json::from_slice::<WhoamiBody>(&body) {
        Ok(body) => Some(WhoamiIdentity {
            subject: CredentialSubject {
                // The wire value stays lenient; unknown types become
                // `SubjectKind::Other` and round-trip verbatim.
                kind: body.subject.kind.into(),
                name: body.subject.name,
            },
            token_name: body.token.name,
            token_prefix: body.token.prefix,
            expires_at: body.token.expires_at,
        }),
        Err(err) => {
            // The body is index metadata about the token, never a secret.
            log::debug!(
                "whoami body was not parsed: {err};\nbody: {}",
                String::from_utf8_lossy(&body)
            );
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
    /// Each rejecting surface with the HTTP status it answered.
    rejected: Vec<(ProbeSurface, u16)>,
    /// The scheme tokens the read surface offered in `WWW-Authenticate`
    /// across both probe legs, verbatim in first-seen order.
    challenge_schemes: Vec<String>,
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

/// Send one probe request, optionally with a forced bearer credential.
/// Returns the response unread (probes never consume bodies; the whoami
/// body is read separately on a 200). The read surface probes with HEAD
/// since only the status matters there; body-consuming probes use GET.
fn probe_request(
    runtime: &tokio::runtime::Runtime,
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &Url,
    bearer: Option<&str>,
) -> Result<reqwest::Response, String> {
    runtime.block_on(async {
        let mut request = client.request(method, url.clone());
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        request.send().await.map_err(crate::utils::format_err)
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

/// Collect into `schemes` the scheme token of every `WWW-Authenticate`
/// challenge on the response, verbatim (original casing) in first-seen
/// order, deduplicated case-insensitively. Splits challenges on
/// top-level commas only, because a naive comma split false-positives
/// on quoted realm values; a comma-continued auth-param
/// (`charset="UTF-8"`) is not a challenge, so a leading token counts as
/// a scheme only when it is a valid HTTP token. (A quoted `\"` is not
/// handled; this only tunes hint messages.)
fn collect_challenge_schemes(headers: &reqwest::header::HeaderMap, schemes: &mut Vec<String>) {
    for value in headers.get_all(reqwest::header::WWW_AUTHENTICATE).iter() {
        let Ok(value) = value.to_str() else { continue };
        // Split into challenges on commas outside double quotes.
        let mut in_quotes = false;
        let challenges = value.split(|c: char| {
            if c == '"' {
                in_quotes = !in_quotes;
            }
            c == ',' && !in_quotes
        });
        for challenge in challenges {
            let token = challenge
                .trim_start()
                .split([' ', '\t'])
                .next()
                .unwrap_or("");
            if token.is_empty() || !token.chars().all(is_tchar) {
                continue;
            }
            if !schemes.iter().any(|s| s.eq_ignore_ascii_case(token)) {
                schemes.push(token.to_string());
            }
        }
    }
}

/// An RFC 9110 `token` character. An auth-param continuation
/// (`charset="UTF-8"`) contains `=`/`"`, which are not tchars, so it is
/// never mistaken for a scheme.
fn is_tchar(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
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
    let baseline = match probe_request(runtime, client, reqwest::Method::HEAD, index_json_url, None)
    {
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
    let mut challenge_schemes = Vec::new();
    collect_challenge_schemes(baseline.headers(), &mut challenge_schemes);
    let forced = match probe_request(
        runtime,
        client,
        reqwest::Method::HEAD,
        index_json_url,
        Some(secret),
    ) {
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
        outcome.rejected.push((surface, forced_status.as_u16()));
        collect_challenge_schemes(forced.headers(), &mut challenge_schemes);
        outcome.challenge_schemes = challenge_schemes;
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
    let response = match probe_request(
        runtime,
        client,
        reqwest::Method::GET,
        &whoami_url,
        Some(secret),
    ) {
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
        outcome.rejected.push((surface, status.as_u16()));
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
                error: crate::utils::format_err(err),
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
    let response = match probe_request(
        runtime,
        &client,
        reqwest::Method::GET,
        &config_url,
        Some(secret),
    ) {
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
                Err(err) => unreachable(crate::utils::format_err(err)),
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
                    error: crate::utils::format_err(err),
                });
                None
            }
        };
    derive_credential_globs(primary_root, endpoints.as_ref())
}

/// Store a bearer credential for the index `index_key` names. The secret
/// arrives as a parameter: a library call never prompts.
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
    index_key: &IndexKey,
    secret: String,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &tokio::runtime::Runtime,
    mut notify: impl FnMut(AuthLoginNotice),
) -> Result<AuthLoginOutcome, AuthCommandError> {
    let key = index_key.as_str().to_string();
    let location = &index_key.location;
    let primary_root = index_key.glob_root.clone();

    // Read the store before any network: an absent keyring backend must
    // be detected before the secret could be spent on a credentialed
    // request. Also detects an existing login for the replacement notice;
    // `list` and `upsert` are separately locked, so that part is
    // best-effort under cross-process races.
    let records = match store.list() {
        Ok(records) => records,
        Err(CredentialStoreError::BackendAbsent { source }) => {
            let globs = guidance_globs(client, location, &primary_root, runtime, &mut notify);
            return Ok(AuthLoginOutcome::BackendUnavailable {
                key,
                globs,
                reason: source.to_string(),
            });
        }
        Err(err) => return Err(err.into()),
    };
    let replacing = records.iter().any(|record| record.key == key);

    let endpoints = fetch_login_endpoints(client, location, &secret, runtime, &mut notify);
    let globs = derive_credential_globs(&primary_root, endpoints.as_ref());

    let outcome =
        run_validation_probes(endpoints.as_ref(), location, &secret, runtime, &mut notify);
    // Refuse only when an exercised surface rejected and none accepted.
    if outcome.accepted.is_empty() && !outcome.rejected.is_empty() {
        return Err(AuthCommandError::ValidationRejected {
            index: key,
            rejected: outcome.rejected,
            challenge_schemes: outcome.challenge_schemes,
        });
    }
    // Stored anyway: warn about each surface that rejected.
    for (surface, status) in &outcome.rejected {
        notify(AuthLoginNotice::SurfaceRejected {
            surface: *surface,
            status: *status,
            challenge_schemes: if *surface == ProbeSurface::Read {
                outcome.challenge_schemes.clone()
            } else {
                Vec::new()
            },
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
            .map(|surface| ValidatedSurface::from(*surface))
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

/// Remove the stored credential for the index `index_key` names.
///
/// Returns the normalized index key the record was stored under. Removing
/// a login that does not exist is an error
/// ([`AuthCommandError::NoStoredCredential`]).
pub fn do_auth_logout<B: BlobBackend>(
    store: &mut LockedBlobStore<B>,
    index_key: &IndexKey,
) -> Result<String, AuthCommandError> {
    let key = index_key.as_str().to_string();
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
    /// `SYSAND_CRED_<LABEL>` stem.
    Env { label: String },
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
        BearerSelection::Ambiguous { matched, .. } => {
            return Err(AuthCommandError::AmbiguousWhoamiCredential {
                url: whoami_url.as_str().to_string(),
                source_name: "`SYSAND_CRED_*` environment variables",
                candidates: crate::commands::publish::dedup_in_order(
                    matched
                        .iter()
                        .map(|entry| format!("SYSAND_CRED_{}", entry.label)),
                ),
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
        BearerSelection::Ambiguous { matched, .. } => {
            Err(AuthCommandError::AmbiguousWhoamiCredential {
                url: whoami_url.as_str().to_string(),
                source_name: "stored credentials",
                candidates: crate::commands::publish::dedup_in_order(
                    matched.iter().map(|entry| format!("`{}`", entry.key())),
                ),
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
    index_key: &IndexKey,
    env_bearers: &GlobMap<EnvBearerAuth>,
    discovery_auth: &P,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &tokio::runtime::Runtime,
) -> Result<AuthWhoamiOutcome, AuthCommandError> {
    let key = index_key.as_str().to_string();

    let endpoints = runtime
        .block_on(fetch_index_config(
            client,
            discovery_auth,
            &index_key.location,
        ))
        .map_err(|err| AuthCommandError::WhoamiDiscoveryFailed {
            index: key.clone(),
            error: crate::utils::format_err(err),
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
        Ok(probe) => match probe_request(
            runtime,
            &probe,
            reqwest::Method::GET,
            &whoami_url,
            Some(&token),
        ) {
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

/// The URL-glob match target of a validated index key
/// ([`location_glob_root`] of its parsed location). `None` when the key
/// does not parse.
fn index_key_glob_root(key: &str) -> Option<String> {
    Some(location_glob_root(&IndexLocation::parse(key).ok()?))
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
