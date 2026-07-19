// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand auth` command orchestration (design/credential-storage.md
//! sections 4, 8, 9, 14): `do_auth_login`, `do_auth_logout`, and
//! `do_auth_status`, generic over [`CredentialStore`].
//!
//! Library calls never prompt and never print; the login secret arrives as
//! a parameter and progress is reported through [`AuthLoginNotice`] values
//! for the host (CLI or bindings) to render. Environment credentials are
//! passed in as [`EnvCredentialEntry`] values: this module does not read
//! the process environment.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use globset::GlobBuilder;
use thiserror::Error;
use url::Url;

use serde::Deserialize;

use crate::{
    auth::Unauthenticated,
    credential_store::{
        CredentialRecord, CredentialScheme, CredentialStore, CredentialStoreError,
        CredentialSubject, normalize_index_key,
    },
    env::discovery::{ResolvedEndpoints, fetch_index_config},
    index_location::{IndexLocation, is_template_syntax},
};

/// Errors from the `sysand auth` commands.
#[derive(Debug, Error)]
pub enum AuthCommandError {
    /// `logout` targeted an index with no stored credential.
    #[error("no stored credential for `{index}`")]
    NoStoredCredential { index: String },
    /// The target is not an HTTP(S) index (for example a `file://` URL).
    #[error("`{url}`: not an HTTP(S) index; nothing to authenticate to")]
    NotHttpIndex { url: String },
    /// The target could not be parsed or normalized as an index URL.
    #[error("invalid index URL for credential storage: {0}")]
    InvalidIndexUrl(String),
    /// The target is a URL-template index location, which `sysand auth`
    /// does not support in v1 (a template does not normalize to a stable
    /// index key). `SYSAND_CRED_*` environment variables remain the
    /// authentication path for templated indexes.
    #[error(
        "`{url}`: URL-template index locations are not supported by `sysand auth`;\n\
         use `SYSAND_CRED_*` environment variables to authenticate a templated index"
    )]
    TemplateIndexUrl { url: String },
    /// Every surface that exercised the credential rejected it and none
    /// accepted it, so nothing was stored (design/credential-storage.md
    /// section 5 refusal rule).
    #[error("{}", validation_rejected_message(.index, .rejected, *.basic_challenge))]
    ValidationRejected {
        /// The normalized index key the login targeted.
        index: String,
        /// The surfaces that exercised and rejected the credential.
        rejected: Vec<ProbeSurface>,
        /// Whether the read surface answered with a `WWW-Authenticate:
        /// Basic` challenge: the index wants username/password, not a
        /// bearer token, and the message routes the user to
        /// `SYSAND_CRED_*` basic credentials (section 5).
        basic_challenge: bool,
    },
    /// The credential store failed.
    #[error(transparent)]
    Store(#[from] CredentialStoreError),
}

fn validation_rejected_message(
    index: &str,
    rejected: &[ProbeSurface],
    basic_challenge: bool,
) -> String {
    let surfaces: Vec<&str> = rejected
        .iter()
        .map(|surface| match surface {
            ProbeSurface::Read => "the index read surface (`index.json`)",
            ProbeSurface::Api => "the index API (`v1/whoami`)",
        })
        .collect();
    let mut message = format!(
        "credential for `{index}` was rejected by {} and accepted by no surface; \
         nothing was stored",
        surfaces.join(" and ")
    );
    if basic_challenge {
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
}

/// A credential-probing surface of an index (design/credential-storage.md
/// section 5): the read surface (`index_root/index.json`) or the API
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

/// Status of one stored login, as shown by `auth status`. Never contains
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
    /// Labels of `SYSAND_CRED_*` entries that may shadow this login.
    ///
    /// Approximate: an env entry is listed when its pattern matches this
    /// record's key URL. Env credentials take precedence per matched
    /// request URL, so an env pattern matching only part of the covered
    /// URLs (or spelled so it misses the key, for example with a port
    /// wildcard) may shadow requests without being listed here.
    pub shadowed_by: Vec<String>,
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

/// Validate that `index_url` is an absolute, non-template HTTP(S) URL and
/// normalize it to its credential store key form.
///
/// Public so the CLI can validate the target before reading a secret (a
/// `file://` or template target must fail before any prompt).
pub fn validated_index_key(index_url: &str) -> Result<String, AuthCommandError> {
    // Reject template syntax before `Url::parse`: the `url` crate would
    // percent-encode the braces and normalization would then silently
    // produce a mangled `%7Bpath%7D` key.
    if is_template_syntax(index_url) {
        return Err(AuthCommandError::TemplateIndexUrl {
            url: index_url.to_string(),
        });
    }
    // Check the scheme before normalizing so a non-HTTP(S) location gets
    // the dedicated message instead of a generic normalization error.
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

/// Progress notice emitted by [`do_auth_login`] while it works, so the
/// host can render it at the right moment (in particular, a replacement is
/// reported strictly before the store write happens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthLoginNotice {
    /// A stored credential for the same key exists and is about to be
    /// overwritten.
    ReplacingExisting { key: String },
    /// The discovery document could not be read (network failure, or an
    /// HTTP error such as 401 on a private index); the credential is
    /// scoped to the URL-derived pattern only.
    DiscoveryUnreachable { error: String },
    /// Discovery resolved a templated `index_root` whose literal prefix
    /// cannot be anchored at a safe URL boundary (at least
    /// `scheme://authority/`), so no glob was derived for it. Deriving one
    /// anyway could produce a pattern matching other hosts.
    TemplateIndexRootSkipped { template: String },
    /// A validation probe was answered with a redirect. Probes never
    /// follow redirects (the verdict would come from a different URL than
    /// the surface nominally probed), so the surface counts as not
    /// tested. `target` names the redirect target when the response
    /// carried one.
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
    /// verdict (design/credential-storage.md section 5): whether it hit
    /// the unauth baseline or the forced retry, the surface counts as not
    /// tested, so rate limiting can never refuse a credential.
    ProbeRateLimited { surface: ProbeSurface },
    /// A surface rejected the credential, but another surface accepted
    /// it, so the credential was stored anyway. `basic_challenge` is true
    /// when the read surface answered with a `WWW-Authenticate: Basic`
    /// challenge (the index wants username/password, not a bearer token).
    SurfaceRejected {
        surface: ProbeSurface,
        basic_challenge: bool,
    },
}

/// Outcome of [`do_auth_login`].
///
/// An absent keyring backend is an outcome rather than an error because
/// the host still needs the derived globs: the CLI prints the exact
/// `SYSAND_CRED_*` lines to set instead (design/credential-storage.md
/// section 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthLoginOutcome {
    /// The credential was persisted.
    Stored {
        key: String,
        globs: Vec<String>,
        /// The surfaces that exercised and accepted the credential, in
        /// probe order (read before api). Empty means "stored, not
        /// validated": either `validation` was disabled or nothing
        /// exercised the credential. Hosts must scope the claim to these
        /// surfaces and never print a bare "validated"
        /// (design/credential-storage.md section 5).
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

/// Derive the escaped URL glob patterns a login for `key` covers
/// (design/credential-storage.md section 8).
///
/// The primary glob anchors on the normalized user-supplied discovery URL
/// (`globset::escape(<root ending in />)` + `**`), so the discovery fetch
/// itself is authenticated on later runs. The resolved `index_root` and
/// `api_root` add further globs only when not already covered by an
/// earlier root (Case B, disjoint host or path); a root that is itself a
/// prefix of an earlier one is also skipped, keeping the set minimal. A
/// templated `index_root` anchors on its literal prefix cut back to the
/// last `/` boundary: under `literal_separator(true)` a `**` crosses `/`
/// only as a full path component, so a mid-segment anchor would not match.
///
/// Both derivation and runtime matching serialize URLs via
/// `url::Url::as_str()`, so IDN/percent-encoding agree on both sides.
fn derive_credential_globs(
    key: &str,
    endpoints: Option<&ResolvedEndpoints>,
    notify: &mut impl FnMut(AuthLoginNotice),
) -> Vec<String> {
    // Roots already covered, each ending in `/`. A candidate root is
    // covered when one of these is its string prefix: the corresponding
    // `<escaped root>**` glob then matches every URL under the candidate
    // (`**` after `/` crosses separators).
    let mut roots: Vec<String> = vec![key.to_string()];

    let push_if_uncovered = |roots: &mut Vec<String>, candidate: &str| {
        if !roots
            .iter()
            .any(|root| candidate.starts_with(root.as_str()))
        {
            // Keep the set minimal in the other direction too: a candidate
            // that is a prefix of an existing root subsumes it.
            roots.retain(|root| !root.starts_with(candidate));
            roots.push(candidate.to_string());
        }
    };

    if let Some(endpoints) = endpoints {
        match &endpoints.index_root {
            IndexLocation::Root(url) => push_if_uncovered(&mut roots, url.as_str()),
            IndexLocation::Template(template) => match template_anchor_root(template.prefix()) {
                Some(anchor) => push_if_uncovered(&mut roots, anchor.as_str()),
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

/// Anchor a templated `index_root`'s literal prefix at a safe URL
/// boundary: the prefix cut back to its last `/`, reparsed as a URL so the
/// anchor uses the same serialization runtime request URLs will (host
/// case, default port). Returns `None` when no anchor at least as deep as
/// `scheme://authority/` exists (for example a placeholder directly in the
/// query of a path-less URL, whose last `/` is the one in `://`).
fn template_anchor_root(prefix: &str) -> Option<Url> {
    let cut = prefix.rfind('/')?;
    let candidate = &prefix[..=cut];
    let url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Some(url)
}

/// Identity fields parsed from a successful `v1/whoami` probe, persisted
/// on the stored record (design/credential-storage.md sections 5, 6, 9).
struct WhoamiIdentity {
    subject: CredentialSubject,
    token_name: Option<String>,
    token_prefix: Option<String>,
    expires_at: Option<DateTime<Utc>>,
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

/// What the validation probes concluded (design/credential-storage.md
/// section 5). A surface appears in `accepted` or `rejected` only when it
/// actually exercised the credential; surfaces that were public,
/// redirected, or unreachable appear in neither (they were reported as
/// notices instead).
#[derive(Default)]
struct ProbeOutcome {
    accepted: Vec<ProbeSurface>,
    rejected: Vec<ProbeSurface>,
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
/// `Basic` scheme. Scheme tokens are case-insensitive (RFC 7235) and one
/// header value can carry several comma-separated challenges; a full
/// challenge parser is overkill, but a plain substring check would
/// false-positive on e.g. `Bearer realm="Basic migration"`.
fn offers_basic_challenge(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value.split(',').any(|part| {
                let token = part.trim_start().split([' ', '\t']).next().unwrap_or("");
                token.eq_ignore_ascii_case("basic")
            })
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
        // Never force-retry a rate-limited surface: the retry would spend
        // more of the rate budget, and a 429 answer to it would prove
        // nothing either (429 is never a verdict, section 5).
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
        // 429 is never a verdict (section 5): a rate-limited forced retry
        // must not count as rejected, or throttling could false-refuse a
        // valid token. A basic challenge seen on the baseline is discarded
        // with the verdict: without a rejection there is nothing to route.
        notify(AuthLoginNotice::ProbeRateLimited { surface });
    } else if forced_status.is_success() {
        outcome.accepted.push(surface);
    } else if forced_status.is_client_error() {
        outcome.rejected.push(surface);
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
        let parsed = runtime
            .block_on(response.bytes())
            .map_err(|err| err.to_string())
            .and_then(|body| {
                serde_json::from_slice::<WhoamiBody>(&body).map_err(|err| err.to_string())
            });
        match parsed {
            Ok(body) => {
                outcome.identity = Some(WhoamiIdentity {
                    subject: CredentialSubject {
                        kind: body.subject.kind,
                        name: body.subject.name,
                    },
                    token_name: body.token.name,
                    token_prefix: body.token.prefix,
                    expires_at: body.token.expires_at,
                });
            }
            // Lenient: the 200 status is the acceptance verdict; a body
            // this client cannot parse only loses the identity fields.
            Err(err) => log::debug!("whoami body was not read: {err}"),
        }
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        outcome.rejected.push(surface);
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // 429 is never a verdict (section 5); the protocol explicitly
        // allows rate limiting on `v1/whoami`.
        notify(AuthLoginNotice::ProbeRateLimited { surface });
    } else {
        notify(AuthLoginNotice::ProbeUnreachable {
            surface,
            error: format!("unexpected status {status}"),
        });
    }
}

/// Run the validation probes (design/credential-storage.md section 5):
/// discovery-first (the already-fetched discovery result is reused; when
/// it was unreachable the read surface falls back to the URL-derived
/// `index.json`, so a fully private index still gets its read surface
/// exercised), and the API surface only when discovery explicitly
/// advertised `api_root` (never the plain-URL runtime default), so a
/// static index is not phantom-probed for an API it does not have.
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

    if let Some(endpoints) = endpoints
        && endpoints.api_root_advertised
        && let Some(api_root) = &endpoints.api_root
    {
        probe_api_surface(runtime, &client, api_root, secret, &mut outcome, notify);
    }

    outcome
}

/// Store a bearer credential for `index_url` (design/credential-storage.md
/// sections 4, 5, 8, 9). The secret arrives as a parameter: a library call
/// never prompts.
///
/// Discovery is fetched best-effort with the unauthenticated policy (no
/// credential exists for the index yet) to resolve `index_root` and
/// `api_root` for glob scoping; when it cannot be read the credential
/// falls back to the URL-derived glob with a
/// [`AuthLoginNotice::DiscoveryUnreachable`] notice. Overwriting an
/// existing record for the same key is reported through
/// [`AuthLoginNotice::ReplacingExisting`] before the write happens (and
/// only once validation has decided the write will happen: a refused
/// login never announces a replacement).
///
/// `validation`: `None` means the default, `true`; the CLI's
/// `--no-validation` flag maps to `Some(false)`, and language bindings can
/// expose this as an optional keyword. When enabled, the section 5 probes
/// run between glob
/// derivation and the store write, and the refusal rule applies: the
/// credential is stored if any exercised surface accepted it, refused
/// ([`AuthCommandError::ValidationRejected`]) when at least one exercised
/// surface rejected it and none accepted, and stored as "not validated"
/// when nothing exercised it. `Some(false)` skips every probe and stores
/// directly. An absent keyring backend returns
/// [`AuthLoginOutcome::BackendUnavailable`] before any probe runs (no
/// network is spent on a credential that cannot be stored), so the
/// `SYSAND_CRED_*` guidance a no-keyring host prints always describes an
/// unvalidated credential.
pub fn do_auth_login<S: CredentialStore>(
    store: &mut S,
    index_url: &str,
    secret: String,
    validation: Option<bool>,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
    mut notify: impl FnMut(AuthLoginNotice),
) -> Result<AuthLoginOutcome, AuthCommandError> {
    let key = validated_index_key(index_url)?;
    let location = IndexLocation::parse(&key)
        .map_err(|err| AuthCommandError::InvalidIndexUrl(format!("`{key}`: {err}")))?;

    let endpoints =
        match runtime.block_on(fetch_index_config(client, &Unauthenticated {}, &location)) {
            Ok(endpoints) => Some(endpoints),
            Err(err) => {
                notify(AuthLoginNotice::DiscoveryUnreachable {
                    error: err.to_string(),
                });
                None
            }
        };
    let globs = derive_credential_globs(&key, endpoints.as_ref(), &mut notify);

    // Detect an existing login before writing, so the host can announce
    // the replacement before the write. `list` and `upsert` are
    // separately locked, so this is best-effort under cross-process
    // races.
    let records = match store.list() {
        Ok(records) => records,
        Err(CredentialStoreError::BackendAbsent { source }) => {
            return Ok(AuthLoginOutcome::BackendUnavailable {
                key,
                globs,
                reason: source.to_string(),
            });
        }
        Err(err) => return Err(err.into()),
    };
    let replacing = records.iter().any(|record| record.key == key);

    // Validation probes (design/credential-storage.md section 5), between
    // glob derivation and the store write.
    let mut validated: Vec<ProbeSurface> = Vec::new();
    let mut identity: Option<WhoamiIdentity> = None;
    if validation.unwrap_or(true) {
        let outcome = run_validation_probes(
            endpoints.as_ref(),
            &location,
            &secret,
            &runtime,
            &mut notify,
        );
        // The refusal rule: refuse only when at least one exercised
        // surface rejected the credential and none accepted it.
        if outcome.accepted.is_empty() && !outcome.rejected.is_empty() {
            return Err(AuthCommandError::ValidationRejected {
                index: key,
                rejected: outcome.rejected,
                basic_challenge: outcome.basic_challenge,
            });
        }
        // Stored anyway (some surface accepted): warn about each surface
        // that rejected.
        for surface in &outcome.rejected {
            notify(AuthLoginNotice::SurfaceRejected {
                surface: *surface,
                basic_challenge: outcome.basic_challenge && *surface == ProbeSurface::Read,
            });
        }
        validated = outcome.accepted;
        identity = outcome.identity;
    }

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

/// Remove the stored login for `index_url`.
///
/// Returns the normalized index key the record was stored under. Removing
/// a login that does not exist is an error
/// ([`AuthCommandError::NoStoredCredential`]).
pub fn do_auth_logout<S: CredentialStore>(
    store: &mut S,
    index_url: &str,
) -> Result<String, AuthCommandError> {
    let key = validated_index_key(index_url)?;
    if store.remove(&key)? {
        Ok(key)
    } else {
        Err(AuthCommandError::NoStoredCredential { index: key })
    }
}

/// Assemble the `auth status` view from stored records and environment
/// entries, against the given clock. Exposed for deterministic tests;
/// [`do_auth_status`] is the store-reading entry point.
pub fn assemble_auth_status(
    records: Vec<CredentialRecord>,
    env: Vec<EnvCredentialEntry>,
    now: DateTime<Utc>,
) -> AuthStatus {
    // Compile each env pattern the same way runtime matching does
    // (`GlobMapBuilder`, `literal_separator(true)`). An invalid pattern
    // cannot shadow anything and is skipped.
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
                key: record.key,
                globs: record.globs,
                expires_at: record.expires_at,
                subject: record.subject,
                token_prefix: record.token_prefix,
                shadowed_by,
            }
        })
        .collect();

    AuthStatus {
        stored: StoredCredentialsStatus::Available(stored),
        env,
    }
}

/// Read the stored logins and assemble the unified `auth status` view.
///
/// An absent keyring backend degrades to the env-only view
/// ([`StoredCredentialsStatus::BackendUnavailable`]); a present but locked
/// or denied backend is a hard error the caller must surface
/// (design/credential-storage.md section 9 taxonomy).
pub fn do_auth_status<S: CredentialStore>(
    store: &S,
    env: Vec<EnvCredentialEntry>,
) -> Result<AuthStatus, AuthCommandError> {
    match store.list() {
        Ok(records) => Ok(assemble_auth_status(records, env, Utc::now())),
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
