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

use crate::{
    auth::Unauthenticated,
    credential_store::{
        CredentialRecord, CredentialScheme, CredentialStore, CredentialStoreError,
        normalize_index_key,
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
    /// The credential store failed.
    #[error(transparent)]
    Store(#[from] CredentialStoreError),
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

/// Status of one stored login, as shown by `auth status`. Never contains
/// the secret.
///
/// Extension point: the validation work (`auth login --validation`) will
/// add identity fields here (`subject`, token `prefix` from `v1/whoami`,
/// design/credential-storage.md section 9) once the record shape carries
/// them; this struct deliberately shows only what the record stores today.
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
    Stored { key: String, globs: Vec<String> },
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

/// Store a bearer credential for `index_url` (design/credential-storage.md
/// sections 4, 8, 9). The secret arrives as a parameter: a library call
/// never prompts.
///
/// Discovery is fetched best-effort with the unauthenticated policy (no
/// credential exists for the index yet) to resolve `index_root` and
/// `api_root` for glob scoping; when it cannot be read the credential
/// falls back to the URL-derived glob with a
/// [`AuthLoginNotice::DiscoveryUnreachable`] notice. Overwriting an
/// existing record for the same key is reported through
/// [`AuthLoginNotice::ReplacingExisting`] before the write happens.
///
/// No validation probe runs here yet: the `--validation` flow
/// (design/credential-storage.md section 5) slots in between glob
/// derivation and the store write.
pub fn do_auth_login<S: CredentialStore>(
    store: &mut S,
    index_url: &str,
    secret: String,
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
    // the replacement first. `list` and `upsert` are separately locked, so
    // this is best-effort under cross-process races.
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
    if records.iter().any(|record| record.key == key) {
        notify(AuthLoginNotice::ReplacingExisting { key: key.clone() });
    }

    // Validation probes (design/credential-storage.md section 5) will run
    // here, between glob derivation and the store write.

    let record = CredentialRecord {
        key: key.clone(),
        globs: globs.clone(),
        scheme: CredentialScheme::Bearer,
        secret,
        expires_at: None,
        extra: serde_json::Map::new(),
    };
    match store.upsert(record) {
        Ok(()) => Ok(AuthLoginOutcome::Stored { key, globs }),
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
                key: record.key,
                globs: record.globs,
                expires_at: record.expires_at,
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
