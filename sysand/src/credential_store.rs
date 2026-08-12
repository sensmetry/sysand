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
//! real use (design/credential-storage.md: "no plaintext credentials
//! file, ever"). It is compiled into release builds only as a
//! hard refusal: a release binary that sees the variable errors rather
//! than silently using the real keyring (which would let a forgotten
//! variable make tests scribble on a developer's keychain).

use std::io;

use camino::Utf8PathBuf;
use sysand_core::project::utils::FsIoError;

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
#[cfg(debug_assertions)]
pub const TEST_STORE_ABSENT: &str = ":absent:";

/// The blob backend the CLI credential store runs on: the OS keyring in
/// real use, a plain file or a simulated-absent backend under the test
/// seam. The plaintext-writing test variants are compiled only in debug
/// builds, so the cleartext path is physically absent from release
/// binaries (not merely refused at runtime).
#[derive(Debug)]
pub enum CliBlobBackend {
    Keyring(OsKeyringBackend),
    /// Test seam: the blob as a plain JSON file at the given path.
    #[cfg(debug_assertions)]
    File(Utf8PathBuf),
    /// Test seam: every access reports an absent keyring backend.
    #[cfg(debug_assertions)]
    Absent,
}

impl BlobBackend for CliBlobBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        match self {
            Self::Keyring(keyring) => keyring.read(),
            #[cfg(debug_assertions)]
            Self::File(path) => match std::fs::read_to_string(path) {
                Ok(raw) => Ok(Some(raw)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(FsIoError::ReadFile(path.clone(), err).into()),
            },
            #[cfg(debug_assertions)]
            Self::Absent => Err(absent()),
        }
    }

    fn write(&self, raw: &str) -> Result<(), CredentialStoreError> {
        match self {
            Self::Keyring(keyring) => keyring.write(raw),
            #[cfg(debug_assertions)]
            Self::File(path) => std::fs::write(path, raw)
                .map_err(|err| FsIoError::WriteFile(path.clone(), err).into()),
            #[cfg(debug_assertions)]
            Self::Absent => Err(absent()),
        }
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        match self {
            Self::Keyring(keyring) => keyring.delete(),
            #[cfg(debug_assertions)]
            Self::File(path) => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(FsIoError::RmFile(path.clone(), err).into()),
            },
            #[cfg(debug_assertions)]
            Self::Absent => Err(absent()),
        }
    }
}

#[cfg(debug_assertions)]
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
    // Debug builds honor the test seam; the file/absent backends only
    // exist here (they are `#[cfg(debug_assertions)]`).
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var(TEST_STORE_ENV_VAR) {
        if value == TEST_STORE_ABSENT {
            // The lock file must not land on the real per-user path
            // either; keep everything test-scoped.
            let lock_path = Utf8PathBuf::from_path_buf(std::env::temp_dir())
                .unwrap_or_else(|p| Utf8PathBuf::from(p.display().to_string()))
                .join(format!("sysand-test-absent-{}.lock", std::process::id()));
            return Ok(LockedBlobStore::new(CliBlobBackend::Absent, lock_path));
        }
        let lock_path = Utf8PathBuf::from(format!("{value}.lock"));
        return Ok(LockedBlobStore::new(
            CliBlobBackend::File(Utf8PathBuf::from(value)),
            lock_path,
        ));
    }
    // Release builds have no test backend at all, so seeing the variable
    // must fail loudly rather than silently use the real keyring (which
    // would let a forgotten variable make tests scribble on a developer's
    // keychain).
    #[cfg(not(debug_assertions))]
    if std::env::var_os(TEST_STORE_ENV_VAR).is_some() {
        return Err(CredentialStoreError::BackendDenied {
            source: format!(
                "{TEST_STORE_ENV_VAR} is set, but this build does not support the\n\
                 test credential store"
            )
            .into(),
        });
    }
    Ok(LockedBlobStore::new(
        CliBlobBackend::Keyring(OsKeyringBackend),
        default_lock_path()?,
    ))
}
