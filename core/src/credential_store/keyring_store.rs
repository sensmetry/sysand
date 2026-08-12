// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The credential store: [`LockedBlobStore`], a persistent store of
//! credential records over a [`BlobBackend`]; the storage design is in
//! design/credential-storage.md.
//!
//! All persisted credentials live in one backend entry holding the
//! versioned JSON blob from the parent module. Read-modify-write is
//! guarded by an exclusive cross-process advisory OS file lock (never an
//! existence-based lock file) and reads by its shared counterpart, both
//! with a bounded wait.
//!
//! The blob handling is generic over [`BlobBackend`] so the lock and
//! fail-closed logic is testable without a real OS
//! keyring, and so hosts can substitute backends (the CLI's debug-only
//! test seam). The production backend, [`OsKeyringBackend`] (one keyring
//! entry: service `sysand`, account `credentials`), sits behind the
//! `keyring` cargo feature; everything else here is unconditional.

use std::fs;
// `PathBuf` feeds only the gated lock-path selection below.
#[cfg(any(feature = "keyring", test))]
use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};
use std::time::{Duration, Instant};

use super::{CredentialBlob, CredentialRecord, CredentialStoreError, parse_blob, serialize_blob};
use crate::project::utils::FsIoError;

/// Keyring service name of the single sysand credential entry.
#[cfg(all(
    feature = "keyring",
    not(all(target_os = "linux", target_env = "musl"))
))]
const KEYRING_SERVICE: &str = "sysand";
/// Keyring account name of the single sysand credential entry.
#[cfg(all(
    feature = "keyring",
    not(all(target_os = "linux", target_env = "musl"))
))]
const KEYRING_ACCOUNT: &str = "credentials";

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
#[cfg(all(
    feature = "keyring",
    not(all(target_os = "linux", target_env = "musl"))
))]
fn map_keyring_error(err: keyring::Error) -> CredentialStoreError {
    match err {
        keyring::Error::PlatformFailure(source) => CredentialStoreError::BackendAbsent { source },
        keyring::Error::NoStorageAccess(source) => CredentialStoreError::BackendDenied { source },
        // The platform store refused the value size (Windows caps
        // credential blobs); size limits are the platform's to enforce.
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
#[cfg(feature = "keyring")]
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyringBackend;

#[cfg(all(
    feature = "keyring",
    not(all(target_os = "linux", target_env = "musl"))
))]
impl OsKeyringBackend {
    fn entry() -> Result<keyring::Entry, CredentialStoreError> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(map_keyring_error)
    }
}

#[cfg(all(
    feature = "keyring",
    not(all(target_os = "linux", target_env = "musl"))
))]
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
/// caller takes the documented `SYSAND_CRED_*` fallback path.
#[cfg(all(feature = "keyring", target_os = "linux", target_env = "musl"))]
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

#[cfg(all(feature = "keyring", target_os = "linux", target_env = "musl"))]
fn musl_backend_absent() -> CredentialStoreError {
    CredentialStoreError::BackendAbsent {
        source: "OS keyring support is not built on musl targets".into(),
    }
}

/// Per-user path for the credential store lock file.
///
/// Linux: `XDG_STATE_HOME` (via `dirs`, defaulting to `~/.local/state`),
/// then the local data dir. macOS and Windows: the local data dir (the
/// state dir is unset there). Both fall back to a dotdir in the home
/// directory. Never a world-writable shared path, and deliberately never
/// the session-scoped `XDG_RUNTIME_DIR`: it is often unset for cron,
/// systemd, and container processes, so two same-user processes with
/// different environments could lock different files while
/// read-modify-writing the same keyring entry, silently losing a record.
#[cfg(feature = "keyring")]
pub fn default_lock_path() -> Result<Utf8PathBuf, CredentialStoreError> {
    lock_path_from_dirs(dirs::state_dir(), dirs::data_local_dir(), dirs::home_dir())
}

/// Pure path selection behind [`default_lock_path`], taking the candidate
/// directories as arguments so the precedence is testable without touching
/// the process environment.
#[cfg(any(feature = "keyring", test))]
fn lock_path_from_dirs(
    state_dir: Option<PathBuf>,
    data_local_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Result<Utf8PathBuf, CredentialStoreError> {
    // A non-UTF-8 candidate directory is exceedingly unlikely (these come
    // from the `dirs` crate); treat one like an unset candidate.
    let utf8 = |dir: Option<PathBuf>| Utf8PathBuf::from_path_buf(dir?).ok();
    if let Some(base) = utf8(state_dir).or_else(|| utf8(data_local_dir)) {
        return Ok(base.join("sysand").join("credentials.lock"));
    }
    match utf8(home_dir) {
        Some(home) => Ok(home.join(".sysand").join("credentials.lock")),
        None => Err(CredentialStoreError::NoLockDir),
    }
}

/// A persistent store of credential records over a [`BlobBackend`],
/// guarding every operation with a cross-process advisory file lock
/// (`flock`/`LockFileEx` via the stable [`std::fs::File`] locking API) and
/// a bounded wait, so each operation is atomic with respect to other
/// processes.
#[derive(Debug)]
pub struct LockedBlobStore<B> {
    backend: B,
    lock_path: Utf8PathBuf,
    lock_timeout: Duration,
    lock_poll_interval: Duration,
}

/// The OS-keyring-backed credential store.
#[cfg(feature = "keyring")]
pub type KeyringCredentialStore = LockedBlobStore<OsKeyringBackend>;

#[cfg(feature = "keyring")]
impl KeyringCredentialStore {
    /// The OS keyring store with the default per-user lock path.
    pub fn open_default() -> Result<Self, CredentialStoreError> {
        Ok(Self::new(OsKeyringBackend, default_lock_path()?))
    }
}

impl<B: BlobBackend> LockedBlobStore<B> {
    pub fn new(backend: B, lock_path: Utf8PathBuf) -> Self {
        Self {
            backend,
            lock_path,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            lock_poll_interval: DEFAULT_LOCK_POLL_INTERVAL,
        }
    }

    /// Override the bounded lock wait (mainly for tests).
    #[must_use]
    pub fn with_lock_timing(mut self, timeout: Duration, poll_interval: Duration) -> Self {
        self.lock_timeout = timeout;
        self.lock_poll_interval = poll_interval;
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
        options
            .open(&self.lock_path)
            .map_err(|err| FsIoError::OpenFile(self.lock_path.clone(), err).into())
    }

    /// Run `body` under the cross-process lock, waiting at most the
    /// configured timeout. Read-only operations take the shared lock so
    /// concurrent readers do not serialize; read-modify-write takes the
    /// exclusive lock.
    fn with_lock<T>(
        &self,
        mode: LockMode,
        body: impl FnOnce(&B) -> Result<T, CredentialStoreError>,
    ) -> Result<T, CredentialStoreError> {
        let file = self.open_lock_file()?;
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            let attempt = match mode {
                LockMode::Shared => file.try_lock_shared(),
                LockMode::Exclusive => file.try_lock(),
            };
            match attempt {
                Ok(()) => {
                    let result = body(&self.backend);
                    // Release explicitly (it would also unlock on `file` drop)
                    if let Err(e) = file.unlock() {
                        log::debug!("failed to unlock lock file `{}`: {e}", self.lock_path);
                    }
                    return result;
                }
                Err(fs::TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(CredentialStoreError::LockTimeout {
                            path: self.lock_path.to_string(),
                        });
                    }
                    std::thread::sleep(self.lock_poll_interval);
                }
                Err(fs::TryLockError::Error(err)) => {
                    return Err(FsIoError::LockFile(self.lock_path.clone(), err).into());
                }
            }
        }
    }
}

/// Which cross-process lock [`LockedBlobStore::with_lock`] takes: shared
/// for read-only operations, exclusive for read-modify-write.
#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

/// Create a directory chain, with the final directory private (0700) on
/// unix.
fn create_private_dir_all(dir: &Utf8Path) -> Result<(), CredentialStoreError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|err| FsIoError::MkDir(dir.to_owned(), err))?;
    Ok(())
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
/// logged-out store keeps the cheap "no entry" fast path. Size limits are
/// the platform's to enforce: an oversized write surfaces as the
/// backend's error (Windows caps blobs, mapped to
/// [`CredentialStoreError::BlobTooLarge`]).
fn store_blob<B: BlobBackend>(
    backend: &B,
    blob: &CredentialBlob,
) -> Result<(), CredentialStoreError> {
    if blob.credentials.is_empty() && blob.extra.is_empty() {
        return backend.delete();
    }
    backend.write(&serialize_blob(blob))
}

impl<B: BlobBackend> LockedBlobStore<B> {
    /// All stored records.
    pub fn list(&self) -> Result<Vec<CredentialRecord>, CredentialStoreError> {
        self.with_lock(LockMode::Shared, |backend| {
            Ok(load_blob(backend)?.credentials)
        })
    }

    /// Insert a record, replacing any existing record with the same `key`.
    pub fn upsert(&mut self, record: CredentialRecord) -> Result<(), CredentialStoreError> {
        self.with_lock(LockMode::Exclusive, |backend| {
            let mut blob = load_blob(backend)?;
            match blob
                .credentials
                .iter_mut()
                .find(|existing| existing.key == record.key)
            {
                Some(existing) => *existing = record,
                None => blob.credentials.push(record),
            }
            store_blob(backend, &blob)
        })
    }

    /// Remove the record with the given `key`. Returns whether a record
    /// was removed.
    pub fn remove(&mut self, key: &str) -> Result<bool, CredentialStoreError> {
        self.with_lock(LockMode::Exclusive, |backend| {
            let mut blob = load_blob(backend)?;
            let before = blob.credentials.len();
            blob.credentials.retain(|record| record.key != key);
            let removed = blob.credentials.len() != before;
            if removed {
                store_blob(backend, &blob)?;
            }
            Ok(removed)
        })
    }
}

// Private tests

#[cfg(test)]
#[path = "./keyring_store_tests.rs"]
mod tests;
