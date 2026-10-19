// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The advisory lock. Two
//! `File`s of one process on one path contend under `flock`, so the
//! second holder here is a second handle, not a second process.

use std::{assert_matches, time::Duration};

use camino_tempfile::tempdir;

use super::*;

#[test]
fn second_exclusive_fails_fast() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let _held = WorkspaceLock::try_acquire(root, LockMode::Exclusive).unwrap();
    let error = WorkspaceLock::try_acquire(root, LockMode::Exclusive).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("another process is using `{root}`")
    );
}

#[test]
fn shared_coexists_with_shared() {
    let dir = tempdir().unwrap();
    let _a = WorkspaceLock::try_acquire(dir.path(), LockMode::Shared).unwrap();
    let b = WorkspaceLock::try_acquire(dir.path(), LockMode::Shared).unwrap();
    assert_eq!(b.mode(), LockMode::Shared);
}

#[test]
fn shared_is_blocked_by_exclusive() {
    let dir = tempdir().unwrap();
    let _held = WorkspaceLock::try_acquire(dir.path(), LockMode::Exclusive).unwrap();
    assert_matches!(
        WorkspaceLock::try_acquire(dir.path(), LockMode::Shared),
        Err(LockError::Held { root: _ })
    );
}

#[test]
fn exclusive_is_blocked_by_shared() {
    let dir = tempdir().unwrap();
    let _held = WorkspaceLock::try_acquire(dir.path(), LockMode::Shared).unwrap();
    assert_matches!(
        WorkspaceLock::try_acquire(dir.path(), LockMode::Exclusive),
        Err(LockError::Held { root: _ })
    );
}

#[test]
fn wait_succeeds_once_released() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let held = WorkspaceLock::try_acquire(&root, LockMode::Exclusive).unwrap();
    let holder = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
    });
    let started = std::time::Instant::now();
    let lock =
        WorkspaceLock::acquire(&root, LockMode::Exclusive, Some(Duration::from_secs(5))).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert_eq!(lock.mode(), LockMode::Exclusive);
    holder.join().unwrap();
}

#[test]
fn wait_expires_with_the_default_text() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let _held = WorkspaceLock::try_acquire(root, LockMode::Exclusive).unwrap();
    let error = WorkspaceLock::acquire(root, LockMode::Exclusive, Some(Duration::from_millis(100)))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("another process is using `{root}`")
    );
}

#[test]
fn drop_releases() {
    let dir = tempdir().unwrap();
    let held = WorkspaceLock::try_acquire(dir.path(), LockMode::Exclusive).unwrap();
    drop(held);
    WorkspaceLock::try_acquire(dir.path(), LockMode::Exclusive).unwrap();
}

#[test]
fn lock_file_is_created_and_kept() {
    let dir = tempdir().unwrap();
    let path = WorkspaceLock::path_for(dir.path());
    assert!(!path.exists());
    {
        let lock = WorkspaceLock::try_acquire(dir.path(), LockMode::Exclusive).unwrap();
        assert_eq!(lock.path(), path);
        assert_eq!(lock.root(), dir.path());
    }
    assert!(path.is_file(), "the lock file stays, empty");
    assert_eq!(std::fs::read(&path).unwrap(), b"");
}
