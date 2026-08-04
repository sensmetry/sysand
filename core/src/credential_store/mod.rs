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
    /// The stored blob failed to parse. This fails closed: a corrupt
    /// blob is never treated as empty, which would clobber all stored
    /// credentials on the next write.
    #[error("credential store unreadable; remove the `sysand` keyring entry to reset")]
    Unreadable,
    /// The stored blob parsed but declares a format version this build
    /// does not know (likely written by a newer sysand). Also fails
    /// closed, but the blob is not corrupt, so no reset is suggested.
    #[error(
        "unsupported credential store blob version {found} (this build supports version {expected})"
    )]
    UnsupportedBlobVersion { found: u32, expected: u32 },
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
    /// I/O failure around the credential store lock file; the inner
    /// error names the operation and path.
    #[error("credential store lock error")]
    Lock(#[from] crate::project::utils::FsIoError),
    /// No per-user directory could be determined for the lock file.
    #[error("could not determine a per-user directory for the credential store lock file")]
    NoLockDir,
    /// The serialized blob exceeds the platform credential size limit.
    #[error(
        "credential store full on this platform (Windows ~2.5 KB limit);\n\
        remove an unused credential or use a smaller token"
    )]
    BlobTooLarge,
}

/// Authentication scheme of a stored credential. v1 stores bearer tokens
/// only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialScheme {
    Bearer,
}

/// The principal type of a [`CredentialSubject`]. A value this build does
/// not know parses as [`SubjectKind::Other`] and round-trips verbatim, so
/// a read-modify-write by an older binary preserves a newer server's type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum SubjectKind {
    User,
    Project,
    Oidc,
    /// An unrecognized principal type, preserved verbatim.
    Other(String),
}

impl SubjectKind {
    /// The canonical string form: the wire form and the display form.
    pub fn as_str(&self) -> &str {
        match self {
            SubjectKind::User => "user",
            SubjectKind::Project => "project",
            SubjectKind::Oidc => "oidc",
            SubjectKind::Other(value) => value,
        }
    }
}

impl From<String> for SubjectKind {
    fn from(value: String) -> Self {
        match value.as_str() {
            "user" => SubjectKind::User,
            "project" => SubjectKind::Project,
            "oidc" => SubjectKind::Oidc,
            _ => SubjectKind::Other(value),
        }
    }
}

impl From<SubjectKind> for String {
    fn from(kind: SubjectKind) -> Self {
        match kind {
            // Moves the unknown value out instead of cloning via `as_str`.
            SubjectKind::Other(value) => value,
            known => known.as_str().to_string(),
        }
    }
}

impl std::fmt::Display for SubjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An index surface that exercised and accepted a credential at login. A
/// value this build does not know parses as [`ValidatedSurface::Other`]
/// and round-trips verbatim, so a surface name written by a newer sysand
/// neither fails the whole blob closed nor gets dropped on rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ValidatedSurface {
    Read,
    Api,
    /// An unrecognized surface name, preserved verbatim.
    Other(String),
}

impl ValidatedSurface {
    /// The canonical string form: the wire form and the display form.
    pub fn as_str(&self) -> &str {
        match self {
            ValidatedSurface::Read => "read",
            ValidatedSurface::Api => "api",
            ValidatedSurface::Other(value) => value,
        }
    }
}

impl From<String> for ValidatedSurface {
    fn from(value: String) -> Self {
        match value.as_str() {
            "read" => ValidatedSurface::Read,
            "api" => ValidatedSurface::Api,
            _ => ValidatedSurface::Other(value),
        }
    }
}

impl From<ValidatedSurface> for String {
    fn from(surface: ValidatedSurface) -> Self {
        match surface {
            // Moves the unknown value out instead of cloning via `as_str`.
            ValidatedSurface::Other(value) => value,
            known => known.as_str().to_string(),
        }
    }
}

impl std::fmt::Display for ValidatedSurface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Identity of the principal a credential authenticates as, learned from
/// `v1/whoami` by a validating login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialSubject {
    /// The principal type; unknown types survive a round-trip
    /// ([`SubjectKind::Other`]).
    #[serde(rename = "type")]
    pub kind: SubjectKind,
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
    /// login, in probe order (`"read"`, `"api"` on the wire). Empty means
    /// "not validated": nothing exercised the credential. Unknown surface
    /// names survive a round-trip ([`ValidatedSurface::Other`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validated: Vec<ValidatedSurface>,
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

/// Parse a credential blob. Both failure modes fail closed (the blob is
/// never silently treated as empty): a parse failure as
/// [`CredentialStoreError::Unreadable`], an unknown `version` as
/// [`CredentialStoreError::UnsupportedBlobVersion`].
pub fn parse_blob(raw: &str) -> Result<CredentialBlob, CredentialStoreError> {
    // Parse error is ignored here, because it may include secrets
    let blob: CredentialBlob =
        serde_json::from_str(raw).map_err(|_| CredentialStoreError::Unreadable)?;
    if blob.version != BLOB_VERSION {
        return Err(CredentialStoreError::UnsupportedBlobVersion {
            found: blob.version,
            expected: BLOB_VERSION,
        });
    }
    Ok(blob)
}

/// Serialize a credential blob to its JSON wire form.
pub fn serialize_blob(blob: &CredentialBlob) -> String {
    // Serialization failure is a bug
    serde_json::to_string(blob).unwrap()
}

/// Test doubles shared by several core test modules (auth, commands::auth,
/// keyring_store), which is why they live here rather than in one test
/// file. Test-only so they never reach the library's public API.
#[cfg(test)]
pub(crate) mod test_support {
    #[cfg(feature = "networking")]
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::CredentialStoreError;
    use super::keyring_store::BlobBackend;
    #[cfg(all(feature = "networking", feature = "filesystem"))]
    use super::keyring_store::LockedBlobStore;

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
    #[cfg(feature = "networking")]
    pub(crate) fn unique_lock_path() -> camino::Utf8PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap_or_else(|p| camino::Utf8PathBuf::from(p.display().to_string()))
            .join(format!(
                "sysand-core-test-{}-{}.lock",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
    }

    /// An empty in-memory store, the test stand-in for the keyring store.
    #[cfg(all(feature = "networking", feature = "filesystem"))]
    pub(crate) fn in_memory_store() -> LockedBlobStore<InMemoryBlobBackend> {
        LockedBlobStore::new(InMemoryBlobBackend::default(), unique_lock_path())
    }
}

// Private tests

#[cfg(test)]
#[path = "./mod_tests.rs"]
mod tests;
