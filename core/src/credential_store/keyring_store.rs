// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! OS-keyring-backed [`CredentialStore`] (design/credential-storage.md §9).
//!
//! All persisted credentials live in one keyring entry (service `sysand`,
//! account `credentials`) holding the versioned JSON blob from the parent
//! module. Read-modify-write is guarded by a cross-process advisory OS
//! file lock (never an existence-based lock file), with a bounded wait.
//!
//! The blob handling is generic over [`BlobBackend`] so the lock,
//! fail-closed, and size-limit logic is testable without a real OS
//! keyring.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    CredentialBlob, CredentialRecord, CredentialStore, CredentialStoreError, parse_blob,
    remove_record, serialize_blob, upsert_record,
};

/// Keyring service name of the single sysand credential entry.
pub(crate) const KEYRING_SERVICE: &str = "sysand";
/// Keyring account name of the single sysand credential entry.
pub(crate) const KEYRING_ACCOUNT: &str = "credentials";

/// Windows `CRED_MAX_CREDENTIAL_BLOB_SIZE`: the Credential Manager caps a
/// credential blob at 2560 bytes, measured against the UTF-16 encoding the
/// keyring crate stores on Windows. Only the Windows enforcement path and
/// the (platform-independent) size-logic tests use it.
#[cfg(any(windows, test))]
pub(crate) const WINDOWS_MAX_BLOB_BYTES: usize = 2560;

const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Storage for the raw credential blob. Implemented by the OS keyring; an
/// in-memory implementation serves headless tests.
pub trait BlobBackend {
    /// The stored blob, or `None` when no entry exists.
    fn read(&self) -> Result<Option<String>, CredentialStoreError>;
    /// Store the blob, replacing any existing entry.
    fn write(&self, raw: &str) -> Result<(), CredentialStoreError>;
    /// Remove the entry entirely. Succeeds when no entry exists, so an
    /// empty store keeps the cheap "no entry" fast path for later reads.
    fn delete(&self) -> Result<(), CredentialStoreError>;
}

/// Map a keyring crate error into the store's error taxonomy.
///
/// The mapping is best-effort: the keyring crate does not cleanly separate
/// "no backend" from "backend refused". `PlatformFailure` (for example no
/// Secret Service daemon to talk to) is treated as an absent backend, so
/// callers may fall back to `SYSAND_CRED_*`; `NoStorageAccess` (locked or
/// denied collection) and other errors must be surfaced.
#[cfg(not(all(target_os = "linux", target_env = "musl")))]
fn map_keyring_error(err: keyring::Error) -> CredentialStoreError {
    match err {
        keyring::Error::PlatformFailure(source) => CredentialStoreError::BackendAbsent { source },
        keyring::Error::NoStorageAccess(source) => CredentialStoreError::BackendDenied { source },
        // The platform store refused the value size (Windows); the blob
        // size gate should catch this first, this is the backstop.
        keyring::Error::TooLong(_, _) => CredentialStoreError::BlobTooLarge,
        // A corrupt or ambiguous entry reads as an unreadable store, with
        // the same reset remediation as a corrupt blob.
        keyring::Error::BadEncoding(_) | keyring::Error::Ambiguous(_) => {
            CredentialStoreError::Unreadable
        }
        other => CredentialStoreError::BackendDenied {
            source: Box::new(other),
        },
    }
}

/// The real OS keyring entry (macOS Keychain, Windows Credential Manager,
/// Linux Secret Service).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyringBackend;

#[cfg(not(all(target_os = "linux", target_env = "musl")))]
impl OsKeyringBackend {
    fn entry() -> Result<keyring::Entry, CredentialStoreError> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(map_keyring_error)
    }
}

#[cfg(not(all(target_os = "linux", target_env = "musl")))]
impl BlobBackend for OsKeyringBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        match Self::entry()?.get_password() {
            Ok(raw) => Ok(Some(raw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_keyring_error(err)),
        }
    }

    fn write(&self, raw: &str) -> Result<(), CredentialStoreError> {
        Self::entry()?.set_password(raw).map_err(map_keyring_error)
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

/// On musl targets the `keyring` crate is not built at all (excluded by
/// policy in Cargo.toml: musl builds are containers/CI where the env-var
/// credential path is the norm). The backend reports "absent", so every
/// caller takes the documented `SYSAND_CRED_*` fallback path
/// (design/credential-storage.md section 9 taxonomy).
#[cfg(all(target_os = "linux", target_env = "musl"))]
impl BlobBackend for OsKeyringBackend {
    fn read(&self) -> Result<Option<String>, CredentialStoreError> {
        Err(musl_backend_absent())
    }

    fn write(&self, _raw: &str) -> Result<(), CredentialStoreError> {
        Err(musl_backend_absent())
    }

    fn delete(&self) -> Result<(), CredentialStoreError> {
        Err(musl_backend_absent())
    }
}

#[cfg(all(target_os = "linux", target_env = "musl"))]
fn musl_backend_absent() -> CredentialStoreError {
    CredentialStoreError::BackendAbsent {
        source: "OS keyring support is not built on musl targets".into(),
    }
}

/// UTF-16 byte length of a string: the unit Windows measures credential
/// blobs in.
pub(crate) fn utf16_byte_len(raw: &str) -> usize {
    raw.encode_utf16().count() * 2
}

/// Check a blob byte length against a platform limit.
pub(crate) fn check_blob_size(byte_len: usize, limit: usize) -> Result<(), CredentialStoreError> {
    if byte_len > limit {
        Err(CredentialStoreError::BlobTooLarge)
    } else {
        Ok(())
    }
}

/// Per-user path for the credential store lock file.
///
/// Unix: `XDG_RUNTIME_DIR`, then `XDG_STATE_HOME` (via `dirs`), then the
/// local data dir (covers macOS, where the XDG dirs are unset), falling
/// back to a dotdir in the home directory. Windows: `%LOCALAPPDATA%`,
/// falling back to the home directory. Never a world-writable shared path.
pub fn default_lock_path() -> Result<PathBuf, CredentialStoreError> {
    #[cfg(windows)]
    let base = dirs::data_local_dir();
    #[cfg(not(windows))]
    let base = dirs::runtime_dir()
        .or_else(dirs::state_dir)
        .or_else(dirs::data_local_dir);

    if let Some(base) = base {
        return Ok(base.join("sysand").join("credentials.lock"));
    }
    match dirs::home_dir() {
        Some(home) => Ok(home.join(".sysand").join("credentials.lock")),
        None => Err(CredentialStoreError::Lock(io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine a per-user directory for the credential store lock file",
        ))),
    }
}

/// A [`CredentialStore`] over a [`BlobBackend`], guarding every operation
/// with a cross-process advisory file lock (`flock`/`LockFileEx` via the
/// stable [`std::fs::File`] locking API) and a bounded wait.
#[derive(Debug)]
pub struct LockedBlobStore<B> {
    backend: B,
    lock_path: PathBuf,
    lock_timeout: Duration,
    lock_poll_interval: Duration,
    /// Max serialized blob size before a write is refused as
    /// [`CredentialStoreError::BlobTooLarge`]. `None` means unbounded;
    /// defaults to the platform limit (only Windows has one). Overridable
    /// so the enforcement path is testable off Windows.
    size_limit: Option<usize>,
}

/// The serialized-blob size limit that applies on this platform, if any.
/// Only Windows caps credential blobs.
fn default_platform_blob_limit() -> Option<usize> {
    #[cfg(windows)]
    {
        Some(WINDOWS_MAX_BLOB_BYTES)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The OS-keyring-backed credential store.
pub type KeyringCredentialStore = LockedBlobStore<OsKeyringBackend>;

impl KeyringCredentialStore {
    /// The OS keyring store with the default per-user lock path.
    pub fn open_default() -> Result<Self, CredentialStoreError> {
        Ok(LockedBlobStore::new(OsKeyringBackend, default_lock_path()?))
    }
}

impl<B: BlobBackend> LockedBlobStore<B> {
    pub fn new(backend: B, lock_path: PathBuf) -> Self {
        LockedBlobStore {
            backend,
            lock_path,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            lock_poll_interval: DEFAULT_LOCK_POLL_INTERVAL,
            size_limit: default_platform_blob_limit(),
        }
    }

    /// Override the bounded lock wait (mainly for tests).
    pub fn with_lock_timing(mut self, timeout: Duration, poll_interval: Duration) -> Self {
        self.lock_timeout = timeout;
        self.lock_poll_interval = poll_interval;
        self
    }

    /// Override the blob size limit so the platform-cap enforcement path
    /// (otherwise Windows-only) can be exercised on any platform.
    #[cfg(test)]
    pub fn with_size_limit(mut self, size_limit: Option<usize>) -> Self {
        self.size_limit = size_limit;
        self
    }

    fn open_lock_file(&self) -> Result<fs::File, CredentialStoreError> {
        if let Some(parent) = self.lock_path.parent() {
            create_private_dir_all(parent)?;
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(options.open(&self.lock_path)?)
    }

    /// Run `body` under the exclusive cross-process lock, waiting at most
    /// the configured timeout.
    fn with_lock<T>(
        &self,
        body: impl FnOnce(&B) -> Result<T, CredentialStoreError>,
    ) -> Result<T, CredentialStoreError> {
        let file = self.open_lock_file()?;
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    let result = body(&self.backend);
                    // Release the advisory lock. The file also unlocks on
                    // drop, matching the previous guard-drop behavior; the
                    // explicit call keeps the release visible and lets the
                    // backend result stand.
                    let _ = file.unlock();
                    return result;
                }
                Err(fs::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(CredentialStoreError::LockTimeout {
                            path: self.lock_path.display().to_string(),
                        });
                    }
                    std::thread::sleep(self.lock_poll_interval);
                }
                Err(fs::TryLockError::Error(err)) => return Err(CredentialStoreError::Lock(err)),
            }
        }
    }
}

/// Create a directory chain, with the final directory private (0700) on
/// unix.
fn create_private_dir_all(dir: &Path) -> Result<(), CredentialStoreError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    Ok(builder.create(dir)?)
}

/// Read and parse the blob; `None` (no entry) is an empty store, but a
/// present-yet-unparsable blob fails closed.
fn load_blob<B: BlobBackend>(backend: &B) -> Result<CredentialBlob, CredentialStoreError> {
    match backend.read()? {
        Some(raw) => parse_blob(&raw),
        None => Ok(CredentialBlob::empty()),
    }
}

/// Write the blob back, deleting the entry when nothing is left so a
/// logged-out store keeps the cheap "no entry" fast path. `size_limit`
/// refuses an oversized serialized blob before the write (the platform cap
/// on Windows).
fn store_blob<B: BlobBackend>(
    backend: &B,
    blob: &CredentialBlob,
    size_limit: Option<usize>,
) -> Result<(), CredentialStoreError> {
    if blob.credentials.is_empty() && blob.extra.is_empty() {
        return backend.delete();
    }
    let raw = serialize_blob(blob)?;
    if let Some(limit) = size_limit {
        check_blob_size(utf16_byte_len(&raw), limit)?;
    }
    backend.write(&raw)
}

impl<B: BlobBackend> CredentialStore for LockedBlobStore<B> {
    fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        self.with_lock(|backend| Ok(load_blob(backend)?.credentials))
    }

    fn upsert(&mut self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        let size_limit = self.size_limit;
        self.with_lock(|backend| {
            let mut blob = load_blob(backend)?;
            upsert_record(&mut blob.credentials, record);
            store_blob(backend, &blob, size_limit)
        })
    }

    fn remove(&mut self, key: &str) -> Result<bool, CredentialStoreError> {
        let size_limit = self.size_limit;
        self.with_lock(|backend| {
            let mut blob = load_blob(backend)?;
            let removed = remove_record(&mut blob.credentials, key);
            if removed {
                store_blob(backend, &blob, size_limit)?;
            }
            Ok(removed)
        })
    }
}

// Private tests

#[cfg(test)]
#[path = "./keyring_store_tests.rs"]
mod tests;
