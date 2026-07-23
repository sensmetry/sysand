// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Persistent credential storage (design/credential-storage.md §9, §14).
//!
//! This module holds the unconditional pieces: the record types, the
//! versioned JSON blob codec, the index-URL key normalization helper, and
//! the [`CredentialStore`] trait, plus an in-memory implementation for
//! tests. The OS-keyring-backed implementation lives in [`keyring_store`]
//! behind the `keyring` cargo feature.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[cfg(feature = "keyring")]
pub mod keyring_store;

/// Current credential blob format version.
pub const BLOB_VERSION: u32 = 1;

/// Errors from the credential store.
///
/// The taxonomy distinguishes an *absent* keyring backend (callers may fall
/// back to `SYSAND_CRED_*` environment variables) from a
/// *present-but-locked/denied* backend (callers must surface the error and
/// never silently degrade).
#[derive(Debug, Error)]
pub enum CredentialStoreError {
    /// The stored blob failed to parse or has an unknown version. This
    /// fails closed: a corrupt blob is never treated as empty, which would
    /// clobber all stored credentials on the next write.
    #[error("credential store unreadable; remove the `sysand` keyring entry to reset")]
    Unreadable,
    /// The given index URL cannot be used as a credential store key.
    #[error("invalid index URL for credential storage: {0}")]
    InvalidIndexUrl(String),
    /// No usable OS keyring backend. Callers can fall back to
    /// `SYSAND_CRED_*` environment variables.
    #[error("no OS keyring backend is available: {source}")]
    BackendAbsent {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The OS keyring exists but refused access (locked or denied).
    /// Callers must surface this, never silently degrade.
    #[error("the OS keyring denied access (it may be locked): {source}")]
    BackendDenied {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Timed out waiting for the cross-process credential store lock.
    #[error(
        "timed out waiting for the credential store lock at `{path}`; retry once other sysand processes finish"
    )]
    LockTimeout { path: String },
    /// I/O failure around the credential store lock file.
    #[error("credential store lock error: {0}")]
    Lock(#[from] std::io::Error),
    /// The serialized blob exceeds the platform credential size limit.
    #[error(
        "credential store full on this platform (Windows ~2.5 KB limit); remove an unused credential or use a smaller token"
    )]
    BlobTooLarge,
    /// Serializing the blob failed.
    #[error("failed to serialize credential store: {0}")]
    Serialize(String),
}

/// Authentication scheme of a stored credential. v1 stores bearer tokens
/// only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialScheme {
    Bearer,
}

/// Identity of the principal a credential authenticates as, learned from
/// `v1/whoami` by a validating login (design/credential-storage.md
/// sections 5, 6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSubject {
    /// The principal type: `user`, `project`, or `oidc`. Kept as a plain
    /// string so a future server-side type survives a round-trip.
    #[serde(rename = "type")]
    pub kind: String,
    /// The principal name: the username for a user token, the project id
    /// for a project token, the publisher identity for an OIDC token.
    pub name: String,
}

/// One stored credential: a normalized index-URL key, the URL glob patterns the
/// credential applies to, the scheme, the secret, plus the identity and
/// expiry fields a validating login learned from `v1/whoami` (absent for
/// non-validated logins and read-only indexes; blob version stays 1, older
/// blobs without them still parse). A validating login also records which
/// surfaces accepted the credential (`validated`), shown by `auth status`.
///
/// Unknown fields written by a newer sysand are preserved in `extra` so a
/// read-modify-write by an older binary does not drop them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRecord {
    pub key: String,
    pub globs: Vec<String>,
    pub scheme: CredentialScheme,
    pub secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Who the credential authenticates as, from `v1/whoami`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<CredentialSubject>,
    /// The user-given token label, from `v1/whoami`. May be absent even
    /// after validation (trusted-publishing tokens have no label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_name: Option<String>,
    /// The token's non-secret display prefix, from `v1/whoami`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_prefix: Option<String>,
    /// The index surfaces that exercised and accepted the credential at
    /// login, in probe order (`"read"`, `"api"`). Empty means "not
    /// validated": nothing exercised the
    /// credential. Plain strings, not an enum, so a surface name written
    /// by a newer sysand parses instead of failing the whole blob closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validated: Vec<String>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The versioned credential blob: what the single keyring entry holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialBlob {
    pub version: u32,
    #[serde(default)]
    pub credentials: Vec<CredentialRecord>,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl CredentialBlob {
    pub fn new(credentials: Vec<CredentialRecord>) -> Self {
        CredentialBlob {
            version: BLOB_VERSION,
            credentials,
            extra: serde_json::Map::new(),
        }
    }

    pub fn empty() -> Self {
        CredentialBlob::new(Vec::new())
    }
}

impl Default for CredentialBlob {
    fn default() -> Self {
        CredentialBlob::empty()
    }
}

/// Parse a credential blob. Any parse failure, or an unknown `version`,
/// fails closed with [`CredentialStoreError::Unreadable`]: a corrupt blob
/// is never silently treated as empty.
pub fn parse_blob(raw: &str) -> Result<CredentialBlob, CredentialStoreError> {
    let blob: CredentialBlob =
        serde_json::from_str(raw).map_err(|_| CredentialStoreError::Unreadable)?;
    if blob.version != BLOB_VERSION {
        return Err(CredentialStoreError::Unreadable);
    }
    Ok(blob)
}

/// Serialize a credential blob to its JSON wire form.
pub fn serialize_blob(blob: &CredentialBlob) -> Result<String, CredentialStoreError> {
    serde_json::to_string(blob).map_err(|err| CredentialStoreError::Serialize(err.to_string()))
}

/// Normalize an index URL for use as a credential record key, so different
/// spellings of the same index do not create duplicate entries
/// (design/credential-storage.md §4).
///
/// Uses the `url` crate deliberately (not `iri_normalize`): §8 requires
/// glob derivation and runtime matching to share `url::Url::as_str()` as
/// their serialization, and the key must agree with the derived globs.
///
/// Normalization: `Url` parsing (lowercased host, punycoded IDN, default
/// port stripped), the scheme restricted to http(s), the path given a
/// trailing slash, and the fragment dropped. URLs carrying a query string
/// or userinfo are rejected: they do not describe an index root.
pub fn normalize_index_key(raw: &str) -> Result<String, CredentialStoreError> {
    let mut url = Url::parse(raw)
        .map_err(|err| CredentialStoreError::InvalidIndexUrl(format!("`{raw}`: {err}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CredentialStoreError::InvalidIndexUrl(format!(
                "`{raw}`: unsupported scheme `{other}`; only http(s) indexes store credentials"
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CredentialStoreError::InvalidIndexUrl(format!(
            "`{raw}`: must not embed userinfo credentials"
        )));
    }
    if url.query().is_some() {
        return Err(CredentialStoreError::InvalidIndexUrl(format!(
            "`{raw}`: must not contain a query string"
        )));
    }
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.into())
}

/// A persistent store of credential records.
///
/// Each method is atomic with respect to other processes: implementations
/// guard read-modify-write internally (the keyring implementation with a
/// cross-process advisory file lock).
pub trait CredentialStore {
    /// All stored records.
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError>;

    /// Insert a record, replacing any existing record with the same `key`.
    fn upsert(&mut self, record: CredentialRecord) -> Result<(), CredentialStoreError>;

    /// Remove the record with the given `key`. Returns whether a record
    /// was removed.
    fn remove(&mut self, key: &str) -> Result<bool, CredentialStoreError>;
}

/// Replace-by-key upsert shared by store implementations (the keyring
/// store and the test-only in-memory store).
#[cfg(any(feature = "keyring", test))]
fn upsert_record(records: &mut Vec<CredentialRecord>, record: CredentialRecord) {
    match records
        .iter_mut()
        .find(|existing| existing.key == record.key)
    {
        Some(existing) => *existing = record,
        None => records.push(record),
    }
}

/// Remove-by-key shared by store implementations (the keyring store and
/// the test-only in-memory store). Returns whether a record was removed.
#[cfg(any(feature = "keyring", test))]
fn remove_record(records: &mut Vec<CredentialRecord>, key: &str) -> bool {
    let before = records.len();
    records.retain(|record| record.key != key);
    records.len() != before
}

/// An in-memory [`CredentialStore`], for tests of code that consumes the
/// trait. Test-only (`#[cfg(test)]`) so this double never reaches the
/// library's public API; several core test modules share it, which is why
/// it lives here rather than in one test file.
#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InMemoryCredentialStore {
    records: Vec<CredentialRecord>,
}

#[cfg(test)]
impl InMemoryCredentialStore {
    pub fn new() -> Self {
        InMemoryCredentialStore::default()
    }
}

#[cfg(test)]
impl CredentialStore for InMemoryCredentialStore {
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        Ok(self.records.clone())
    }

    fn upsert(&mut self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        upsert_record(&mut self.records, record);
        Ok(())
    }

    fn remove(&mut self, key: &str) -> Result<bool, CredentialStoreError> {
        Ok(remove_record(&mut self.records, key))
    }
}

// Private tests

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
