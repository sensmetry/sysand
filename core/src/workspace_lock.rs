// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! The advisory lock a mutating command holds on a workspace
//! (`workspaces.md` "The advisory lock").
//!
//! One file, `.sysand/.lock`, inside the environment directory at the
//! workspace root -- the project root for a standalone project. Acquiring
//! the lock creates `.sysand/` if it is absent; an environment is present
//! only once `.sysand/env.toml` exists, so a `.sysand/` holding nothing but
//! the lock file reads as no environment. The file holds no content and
//! is never deleted; the operating system releases the lock when its
//! holder exits, so a lock is never stale.
//!
//! The mechanism is `std::fs::File::try_lock`/`try_lock_shared`; no
//! crate. Writers take it
//! exclusively, `sync --verify` takes it shared so it never reads a torn
//! state. It is cross-process only: two operations in one embedding
//! process are not its business.
//!
//! The CLI acquires the lock once at the top of a mutating command and
//! hands the handle to the environment it writes
//! ([`crate::env::local_directory::LocalDirectoryEnvironment::hold`]);
//! an embedder does the same itself. Nothing relies on re-entrant locking.

use std::{
    fs,
    io::ErrorKind,
    sync::Arc,
    time::{Duration, Instant},
};

use camino::{Utf8Path, Utf8PathBuf};
use thiserror::Error;

use crate::{env::DEFAULT_ENV_NAME, project::utils::FsIoError};

/// The lock file's name, inside the environment directory
/// ([`DEFAULT_ENV_NAME`]) at the workspace root.
pub const LOCK_FILE_NAME: &str = ".lock";

/// How long `acquire` sleeps between attempts while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Shared for readers that must not see a torn state (`sync --verify`),
/// exclusive for writers.
pub use crate::project::utils::LockMode;

#[derive(Error, Debug)]
pub enum LockError {
    /// Another process holds the lock. "process",
    /// not "`sysand` process": the holder may be an embedder.
    #[error("another process is using `{root}`")]
    Held { root: Utf8PathBuf },
    #[error(transparent)]
    Io(#[from] Box<FsIoError>),
}

/// A held advisory lock on a workspace; released on drop.
#[derive(Debug)]
pub struct WorkspaceLock {
    file: fs::File,
    root: Utf8PathBuf,
    path: Utf8PathBuf,
    mode: LockMode,
}

impl WorkspaceLock {
    /// Where the lock file of the workspace at `root` lives.
    pub fn path_for(root: &Utf8Path) -> Utf8PathBuf {
        root.join(DEFAULT_ENV_NAME).join(LOCK_FILE_NAME)
    }

    /// Take the lock now or fail with [`LockError::Held`].
    pub fn try_acquire(root: &Utf8Path, mode: LockMode) -> Result<Self, LockError> {
        Self::acquire(root, mode, None)
    }

    /// Take the lock, waiting up to `wait` for the holder to release it
    /// (polling), or failing at once with [`LockError::Held`] when `wait`
    /// is `None`. While waiting, the waiting text is logged once.
    pub fn acquire(
        root: &Utf8Path,
        mode: LockMode,
        wait: Option<Duration>,
    ) -> Result<Self, LockError> {
        let path = Self::path_for(root);
        let env_dir = root.join(DEFAULT_ENV_NAME);
        match fs::create_dir(&env_dir) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(err) => return Err(Box::new(FsIoError::MkDir(env_dir, err)).into()),
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|err| Box::new(FsIoError::OpenFile(path.clone(), err)))?;
        let deadline = wait.map(|wait| Instant::now() + wait);
        let mut announced = false;
        loop {
            match mode.try_lock(&file) {
                Ok(()) => {
                    log::debug!("holding the {mode:?} lock on `{root}` (`{path}`)");
                    return Ok(Self {
                        file,
                        root: root.to_path_buf(),
                        path,
                        mode,
                    });
                }
                Err(fs::TryLockError::WouldBlock) => {
                    let Some(deadline) = deadline else {
                        return Err(LockError::Held {
                            root: root.to_path_buf(),
                        });
                    };
                    if Instant::now() >= deadline {
                        return Err(LockError::Held {
                            root: root.to_path_buf(),
                        });
                    }
                    if !announced {
                        announced = true;
                        log::info!(
                            "{:>12} another process is using `{root}`; waiting up to {} seconds",
                            ' ',
                            wait.map_or(0, |w| w.as_secs())
                        );
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(fs::TryLockError::Error(err)) => {
                    return Err(Box::new(FsIoError::LockFile(path, err)).into());
                }
            }
        }
    }

    pub fn mode(&self) -> LockMode {
        self.mode
    }

    /// The workspace root the lock is on.
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    /// The lock file.
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }
}

/// A [`WorkspaceLock`] taken in [`LockMode::Exclusive`], the only mode a
/// writer may hold.
#[derive(Debug, Clone)]
pub struct ExclusiveLock(Arc<WorkspaceLock>);

impl ExclusiveLock {
    /// `None` when `lock` is shared.
    pub fn new(lock: Arc<WorkspaceLock>) -> Option<Self> {
        (lock.mode == LockMode::Exclusive).then_some(Self(lock))
    }

    pub fn root(&self) -> &Utf8Path {
        self.0.root()
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        // Dropping the file would release it too; explicit so a failure
        // is at least visible.
        if let Err(err) = self.file.unlock() {
            log::debug!("failed to unlock `{}`: {err}", self.path);
        }
    }
}

#[cfg(test)]
#[path = "./workspace_lock_tests.rs"]
mod tests;
