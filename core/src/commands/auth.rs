// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand auth` command orchestration (design/credential-storage.md
//! sections 4, 9, 14): `do_auth_logout` and `do_auth_status`, generic over
//! [`CredentialStore`].
//!
//! Library calls never prompt and never print; they return data for the
//! host (CLI or bindings) to render. Environment credentials are passed in
//! as [`EnvCredentialEntry`] values: this module does not read the process
//! environment.

use chrono::{DateTime, Utc};
use globset::GlobBuilder;
use thiserror::Error;
use url::Url;

use crate::credential_store::{
    CredentialRecord, CredentialStore, CredentialStoreError, normalize_index_key,
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

/// Validate that `index_url` is an absolute HTTP(S) URL and normalize it
/// to its credential store key form.
fn index_key_for(index_url: &str) -> Result<String, AuthCommandError> {
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

/// Remove the stored login for `index_url`.
///
/// Returns the normalized index key the record was stored under. Removing
/// a login that does not exist is an error
/// ([`AuthCommandError::NoStoredCredential`]).
pub fn do_auth_logout<S: CredentialStore>(
    store: &mut S,
    index_url: &str,
) -> Result<String, AuthCommandError> {
    let key = index_key_for(index_url)?;
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
