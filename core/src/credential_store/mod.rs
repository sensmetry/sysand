// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Persistent credential storage; the full design is in
//! design/credential-storage.md.
//!
//! This module holds the unconditional pieces: the record types, the
//! versioned JSON blob codec, and the index-URL key normalization helper.
//! The store itself is [`keyring_store::LockedBlobStore`], generic over
//! the [`keyring_store::BlobBackend`] storage seam; the OS-keyring backend
//! lives behind the `keyring` cargo feature.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

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
    #[error("the OS keyring denied access (it may be locked)")]
    BackendDenied {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Timed out waiting for the cross-process credential store lock.
    #[error(
        "timed out waiting for the credential store lock at `{path}`; retry once other sysand processes finish"
    )]
    LockTimeout { path: String },
    /// I/O failure around the credential store lock file.
    #[error("credential store lock error")]
    Lock(#[from] std::io::Error),
    /// The serialized blob exceeds the platform credential size limit.
    #[error(
        "credential store full on this platform (Windows ~2.5 KB limit);\n\
        remove an unused credential or use a smaller token"
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
/// `v1/whoami` by a validating login.
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
/// expiry fields a validating login learned from `v1/whoami`. A validating login also
/// records which surfaces accepted the credential (`validated`), shown by `auth status`.
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
/// spellings of the same index do not create duplicate entries.
///
/// Uses the `url` crate deliberately (not `iri_normalize`): glob
/// derivation and runtime matching share `url::Url::as_str()` as their
/// serialization, and the key must agree with the derived globs.
///
/// Normalization: `Url` parsing (lowercased host, punycoded IDN, default
/// port stripped), the scheme restricted to http(s), the path given a
/// trailing slash, and the fragment dropped. URLs carrying a query string
/// or userinfo are rejected: they do not describe an index root.
pub fn normalize_index_key(raw: &str) -> Result<String, CredentialStoreError> {
    let mut url = Url::parse(raw).map_err(|err| {
        CredentialStoreError::InvalidIndexUrl(format!("`{}`: {err}", redact_userinfo(raw)))
    })?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CredentialStoreError::InvalidIndexUrl(format!(
                "`{}`: unsupported scheme `{other}`; only http(s) indexes store credentials",
                redact_userinfo(raw)
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CredentialStoreError::InvalidIndexUrl(format!(
            "`{}`: must not embed userinfo credentials",
            redact_userinfo(raw)
        )));
    }
    if url.query().is_some() {
        return Err(CredentialStoreError::InvalidIndexUrl(format!(
            "`{raw}`: must not contain a query string"
        )));
    }
    url.set_fragment(None);
    url.path_segments_mut()
        .expect("http(s) URLs always have path segments")
        .pop_if_empty()
        .push("");
    Ok(url.into())
}

/// Render a possibly malformed URL for an error message with any userinfo
/// replaced by `<redacted>`, so an embedded password never reaches stderr
/// or CI logs. String-based on purpose: it must work for inputs
/// `Url::parse` rejected.
fn redact_userinfo(raw: &str) -> std::borrow::Cow<'_, str> {
    // The authority runs from after any scheme separator to the first
    // `/`, `?`, or `#`; userinfo is everything up to the last `@` in it.
    let authority_start = raw.find("://").map_or(0, |idx| idx + 3);
    let authority_end = raw[authority_start..]
        .find(['/', '?', '#'])
        .map_or(raw.len(), |idx| authority_start + idx);
    match raw[authority_start..authority_end].rfind('@') {
        Some(at) => std::borrow::Cow::Owned(format!(
            "{}<redacted>@{}",
            &raw[..authority_start],
            &raw[authority_start + at + 1..]
        )),
        None => std::borrow::Cow::Borrowed(raw),
    }
}

/// Test doubles shared by several core test modules (auth, commands::auth,
/// keyring_store), which is why they live here rather than in one test
/// file. Test-only so they never reach the library's public API.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    };

    use super::CredentialStoreError;
    use super::keyring_store::{BlobBackend, LockedBlobStore};

    /// In-memory [`BlobBackend`] sharing its contents across clones,
    /// standing in for the single OS keyring entry.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct InMemoryBlobBackend {
        blob: Arc<Mutex<Option<String>>>,
    }

    impl InMemoryBlobBackend {
        pub(crate) fn with_contents(raw: &str) -> Self {
            InMemoryBlobBackend {
                blob: Arc::new(Mutex::new(Some(raw.to_string()))),
            }
        }

        pub(crate) fn contents(&self) -> Option<String> {
            self.blob.lock().unwrap().clone()
        }
    }

    impl BlobBackend for InMemoryBlobBackend {
        fn read(&self) -> Result<Option<String>, CredentialStoreError> {
            Ok(self.blob.lock().unwrap().clone())
        }

        fn write(&self, raw: &str) -> Result<(), CredentialStoreError> {
            *self.blob.lock().unwrap() = Some(raw.to_string());
            Ok(())
        }

        fn delete(&self) -> Result<(), CredentialStoreError> {
            *self.blob.lock().unwrap() = None;
            Ok(())
        }
    }

    /// A lock-file path no other store in this test process shares, so
    /// parallel tests never contend on the cross-process lock.
    pub(crate) fn unique_lock_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sysand-core-test-{}-{}.lock",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// An empty in-memory store, the test stand-in for the keyring store.
    pub(crate) fn in_memory_store() -> LockedBlobStore<InMemoryBlobBackend> {
        LockedBlobStore::new(InMemoryBlobBackend::default(), unique_lock_path())
    }
}

// Private tests

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
