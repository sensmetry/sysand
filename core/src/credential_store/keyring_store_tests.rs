// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Tests of the locked blob store logic against an in-memory backend, so
//! they run headlessly without a real OS keyring.

use std::time::Duration;

use camino_tempfile::tempdir;

use std::path::PathBuf;

use super::{
    BlobBackend, LockedBlobStore, WINDOWS_MAX_BLOB_BYTES, check_blob_size, lock_path_from_dirs,
    utf16_byte_len,
};
use crate::credential_store::test_support::InMemoryBlobBackend;
use crate::credential_store::{CredentialRecord, CredentialScheme, CredentialStoreError};

fn record(key: &str, secret: &str) -> CredentialRecord {
    CredentialRecord {
        key: key.to_string(),
        globs: vec![format!("{key}**")],
        scheme: CredentialScheme::Bearer,
        secret: secret.to_string(),
        expires_at: None,
        subject: None,
        token_name: None,
        token_prefix: None,
        validated: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn store_at(
    backend: InMemoryBlobBackend,
    dir: &camino_tempfile::Utf8TempDir,
) -> LockedBlobStore<InMemoryBlobBackend> {
    LockedBlobStore::new(backend, dir.path().join("credentials.lock").into())
}

#[test]
fn read_modify_write_persists_records() {
    let dir = tempdir().unwrap();
    let backend = InMemoryBlobBackend::default();
    let mut store = store_at(backend.clone(), &dir);

    assert!(store.list().unwrap().is_empty());
    store.upsert(record("https://a.example/", "tok-a")).unwrap();
    store.upsert(record("https://b.example/", "tok-b")).unwrap();
    store
        .upsert(record("https://a.example/", "tok-a2"))
        .unwrap();

    // A second store over the same backend sees the same records.
    let other = store_at(backend.clone(), &dir);
    let records = other.list().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].secret, "tok-a2");

    let mut store = other;
    assert!(store.remove("https://a.example/").unwrap());
    assert!(!store.remove("https://a.example/").unwrap());
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(store.list().unwrap()[0].key, "https://b.example/");

    // Removing the last record deletes the backend entry outright.
    assert!(store.remove("https://b.example/").unwrap());
    assert_eq!(
        backend.contents(),
        None,
        "empty store must delete the entry"
    );
}

#[test]
fn corrupt_blob_fails_closed_and_is_not_clobbered() {
    let dir = tempdir().unwrap();
    let backend = InMemoryBlobBackend::with_contents("not json at all");
    let mut store = store_at(backend.clone(), &dir);

    assert!(matches!(
        store.list().unwrap_err(),
        CredentialStoreError::Unreadable
    ));
    assert!(matches!(
        store
            .upsert(record("https://a.example/", "tok"))
            .unwrap_err(),
        CredentialStoreError::Unreadable
    ));
    assert!(matches!(
        store.remove("https://a.example/").unwrap_err(),
        CredentialStoreError::Unreadable
    ));
    assert_eq!(
        backend.contents().as_deref(),
        Some("not json at all"),
        "a failed operation must never overwrite the stored blob"
    );
}

#[test]
fn lock_wait_is_bounded() {
    let dir = tempdir().unwrap();
    let lock_path: std::path::PathBuf = dir.path().join("credentials.lock").into();
    let mut store = LockedBlobStore::new(InMemoryBlobBackend::default(), lock_path.clone())
        .with_lock_timing(Duration::from_millis(100), Duration::from_millis(10));

    // Hold the advisory lock on a separate file descriptor; flock and
    // LockFileEx contend per open file description, also in-process.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    file.try_lock().unwrap();

    let err = store
        .upsert(record("https://a.example/", "tok"))
        .unwrap_err();
    assert!(matches!(err, CredentialStoreError::LockTimeout { .. }));
    file.unlock().unwrap();

    // Once released, the operation succeeds.
    store.upsert(record("https://a.example/", "tok")).unwrap();
}

#[test]
fn lock_path_prefers_state_then_data_local_then_home() {
    // Pure precedence check: no process-env mutation. The session-scoped
    // XDG_RUNTIME_DIR is deliberately not a candidate (two same-user
    // processes with different environments must lock the same file).
    let state = Some(PathBuf::from("/xdg/state"));
    let data_local = Some(PathBuf::from("/xdg/data-local"));
    let home = Some(PathBuf::from("/home/user"));

    assert_eq!(
        lock_path_from_dirs(state.clone(), data_local.clone(), home.clone()).unwrap(),
        PathBuf::from("/xdg/state/sysand/credentials.lock")
    );
    assert_eq!(
        lock_path_from_dirs(None, data_local, home.clone()).unwrap(),
        PathBuf::from("/xdg/data-local/sysand/credentials.lock")
    );
    assert_eq!(
        lock_path_from_dirs(None, None, home).unwrap(),
        PathBuf::from("/home/user/.sysand/credentials.lock")
    );
    assert!(matches!(
        lock_path_from_dirs(None, None, None).unwrap_err(),
        CredentialStoreError::Lock(_)
    ));
}

#[test]
fn shared_lock_lets_reads_through_but_blocks_writes() {
    let dir = tempdir().unwrap();
    let lock_path: std::path::PathBuf = dir.path().join("credentials.lock").into();
    let mut store = LockedBlobStore::new(InMemoryBlobBackend::default(), lock_path.clone())
        .with_lock_timing(Duration::from_millis(100), Duration::from_millis(10));
    store.upsert(record("https://a.example/", "tok")).unwrap();

    // Hold a shared lock on a separate file descriptor, like a concurrent
    // `list` would.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    file.try_lock_shared().unwrap();

    // Reads take the shared lock and proceed alongside another reader...
    assert_eq!(store.list().unwrap().len(), 1);
    // ...while read-modify-write needs the exclusive lock and times out.
    let err = store
        .upsert(record("https://b.example/", "tok"))
        .unwrap_err();
    assert!(matches!(err, CredentialStoreError::LockTimeout { .. }));
    file.unlock().unwrap();
}

#[test]
fn exclusive_lock_blocks_reads() {
    let dir = tempdir().unwrap();
    let lock_path: std::path::PathBuf = dir.path().join("credentials.lock").into();
    let store = LockedBlobStore::new(InMemoryBlobBackend::default(), lock_path.clone())
        .with_lock_timing(Duration::from_millis(100), Duration::from_millis(10));

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    file.try_lock().unwrap();

    // A writer holds the exclusive lock: the shared read must wait it out
    // (bounded), never read a half-written store.
    let err = store.list().unwrap_err();
    assert!(matches!(err, CredentialStoreError::LockTimeout { .. }));
    file.unlock().unwrap();
    assert!(store.list().unwrap().is_empty());
}

#[cfg(not(all(target_os = "linux", target_env = "musl")))]
#[test]
fn map_keyring_error_covers_the_taxonomy() {
    use super::map_keyring_error;

    // `PlatformFailure` (for example no Secret Service daemon) means "no
    // usable backend": callers may fall back to `SYSAND_CRED_*`.
    assert!(matches!(
        map_keyring_error(keyring::Error::PlatformFailure("no daemon".into())),
        CredentialStoreError::BackendAbsent { .. }
    ));
    // A present-but-locked/denied backend must be surfaced.
    assert!(matches!(
        map_keyring_error(keyring::Error::NoStorageAccess("locked".into())),
        CredentialStoreError::BackendDenied { .. }
    ));
    // The platform size backstop maps to the same error as the size gate.
    assert!(matches!(
        map_keyring_error(keyring::Error::TooLong("secret".to_string(), 2560)),
        CredentialStoreError::BlobTooLarge
    ));
    // Corrupt or ambiguous entries read as an unreadable store.
    assert!(matches!(
        map_keyring_error(keyring::Error::BadEncoding(vec![0xff])),
        CredentialStoreError::Unreadable
    ));
    assert!(matches!(
        map_keyring_error(keyring::Error::Ambiguous(Vec::new())),
        CredentialStoreError::Unreadable
    ));
    // Anything else is surfaced as a denied backend, never a fallback.
    assert!(matches!(
        map_keyring_error(keyring::Error::NoDefaultStore),
        CredentialStoreError::BackendDenied { .. }
    ));
}

#[test]
fn concurrent_writers_do_not_lose_records() {
    let dir = tempdir().unwrap();
    let backend = InMemoryBlobBackend::default();
    let lock_path: std::path::PathBuf = dir.path().join("credentials.lock").into();

    // 4 writers x 2 records: enough to lose an update without the lock,
    // while the final blob stays under the Windows UTF-16 size cap, which
    // is enforced on every write there (8 short records is ~900 of the
    // 1280 allowed UTF-16 units).
    let handles: Vec<_> = (0..4)
        .map(|writer| {
            let backend = backend.clone();
            let lock_path = lock_path.clone();
            std::thread::spawn(move || {
                let mut store = LockedBlobStore::new(backend, lock_path);
                for i in 0..2 {
                    store
                        .upsert(record(&format!("https://w{writer}.example/{i}/"), "tok"))
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let store = store_at(backend, &dir);
    assert_eq!(store.list().unwrap().len(), 8);
}

#[test]
fn blob_size_check_uses_utf16_length() {
    // 1300 ASCII chars fit in UTF-8 under 2560 bytes but exceed the
    // Windows UTF-16 limit.
    let blob = "a".repeat(1300);
    assert_eq!(utf16_byte_len(&blob), 2600);
    let err = check_blob_size(utf16_byte_len(&blob), WINDOWS_MAX_BLOB_BYTES).unwrap_err();
    assert!(matches!(err, CredentialStoreError::BlobTooLarge));
    assert!(
        err.to_string().contains("Windows"),
        "message must name the platform limit: {err}"
    );

    let small = "a".repeat(1280);
    check_blob_size(utf16_byte_len(&small), WINDOWS_MAX_BLOB_BYTES).unwrap();
}

#[test]
fn upsert_enforces_the_size_limit_and_does_not_write_when_exceeded() {
    // The platform-cap enforcement (store_blob -> upsert) is Windows-only
    // in production, so CI never runs it. Inject a tiny limit to drive the
    // same wiring on any platform: prove the gate is connected, not just
    // that the `check_blob_size` helper works, and that an over-limit write
    // is refused before it reaches the backend.
    let dir = tempdir().unwrap();
    let backend = InMemoryBlobBackend::default();
    let mut store = store_at(backend.clone(), &dir).with_size_limit(Some(16));

    let err = store
        .upsert(record(
            "https://a.example/",
            "a-token-well-over-sixteen-bytes",
        ))
        .unwrap_err();
    assert!(matches!(err, CredentialStoreError::BlobTooLarge));
    // The oversized blob never reached the backend.
    assert!(backend.read().unwrap().is_none());

    // A generous limit lets the same write through, so the gate is the
    // size check, not an unconditional refusal.
    let mut ok_store = store_at(backend.clone(), &dir).with_size_limit(Some(100_000));
    ok_store
        .upsert(record("https://a.example/", "tok"))
        .unwrap();
    assert!(backend.read().unwrap().is_some());
}
