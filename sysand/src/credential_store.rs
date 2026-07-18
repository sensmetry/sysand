// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The CLI's credential store construction, including a deliberate test
//! seam: CLI integration tests must never touch the real OS keyring
//! (macOS prompts per test; CI runners have no Secret Service), so in
//! debug builds the [`TEST_STORE_ENV_VAR`] environment variable can swap
//! the OS keyring for a plain-file blob backend.
//!
//! The test seam is exactly that: a test seam. The file backend stores
//! the blob (including secrets) in plaintext, which the design forbids for
//! real use (design/credential-storage.md section 9, "no plaintext
//! credentials file, ever"). It is compiled into release builds only as a
//! hard refusal: a release binary that sees the variable errors rather
//! than silently using the real keyring (which would let a forgotten
//! variable make tests scribble on a developer's keychain).

use std::io;
use std::path::PathBuf;

use sysand_core::credential_store::{
    CredentialStoreError,
    keyring_store::{BlobBackend, LockedBlobStore, OsKeyringBackend, default_lock_path},
};

/// Debug-build-only override selecting the credential store backend for
/// tests: a path selects a plain-file blob backend (lock file at
/// `<path>.lock`), the special value `:absent:` simulates a host with no
/// keyring backend. Not a supported production path.
pub const TEST_STORE_ENV_VAR: &str = "SYSAND_TEST_CREDENTIAL_STORE";

/// Special [`TEST_STORE_ENV_VAR`] value simulating an absent keyring
/// backend.
pub const TEST_STORE_ABSENT: &str = ":absent:";

/// The blob backend the CLI credential store runs on: the OS keyring in
/// real use, a plain file or a simulated-absent backend under the test
/// seam.
#[derive(Debug)]
pub enum CliBlobBackend {
    Keyring(OsKeyringBackend),
    /// Test seam: the blob as a plain JSON file at the given path.
    File(PathBuf),
    /// Test seam: every access reports an absent keyring backend.
    Absent,
}

impl BlobBackend for CliBlobBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        match self {
            CliBlobBackend::Keyring(keyring) => keyring.read(),
            CliBlobBackend::File(path) => match std::fs::read_to_string(path) {
                Ok(raw) => Ok(Some(raw)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(CredentialStoreError::Lock(err)),
            },
            CliBlobBackend::Absent => Err(absent()),
        }
    }

    fn write(&self, raw: &str) -> Result<(), CredentialStoreError> {
        match self {
            CliBlobBackend::Keyring(keyring) => keyring.write(raw),
            CliBlobBackend::File(path) => {
                std::fs::write(path, raw).map_err(CredentialStoreError::Lock)
            }
            CliBlobBackend::Absent => Err(absent()),
        }
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        match self {
            CliBlobBackend::Keyring(keyring) => keyring.delete(),
            CliBlobBackend::File(path) => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(CredentialStoreError::Lock(err)),
            },
            CliBlobBackend::Absent => Err(absent()),
        }
    }
}

fn absent() -> CredentialStoreError {
    CredentialStoreError::BackendAbsent {
        source: "simulated absent backend (test seam)".into(),
    }
}

/// The credential store type every CLI command (and the lazy auth policy)
/// uses.
pub type CliCredentialStore = LockedBlobStore<CliBlobBackend>;

/// Open the CLI credential store: the OS keyring, unless the debug-only
/// test seam ([`TEST_STORE_ENV_VAR`]) selects a file-backed or
/// simulated-absent store.
pub fn open_cli_credential_store() -> Result<CliCredentialStore, CredentialStoreError> {
    if let Ok(value) = std::env::var(TEST_STORE_ENV_VAR) {
        if !cfg!(debug_assertions) {
            // Fail loudly: silently falling through to the OS keyring
            // would defeat the seam's whole purpose.
            return Err(CredentialStoreError::Lock(io::Error::other(format!(
                "{TEST_STORE_ENV_VAR} is set, but this build does not support the \
                 test credential store"
            ))));
        }
        if value == TEST_STORE_ABSENT {
            // The lock file must not land on the real per-user path
            // either; keep everything test-scoped.
            let lock_path = std::env::temp_dir()
                .join(format!("sysand-test-absent-{}.lock", std::process::id()));
            return Ok(LockedBlobStore::new(CliBlobBackend::Absent, lock_path));
        }
        let lock_path = PathBuf::from(format!("{value}.lock"));
        return Ok(LockedBlobStore::new(
            CliBlobBackend::File(PathBuf::from(value)),
            lock_path,
        ));
    }
    Ok(LockedBlobStore::new(
        CliBlobBackend::Keyring(OsKeyringBackend),
        default_lock_path()?,
    ))
}
