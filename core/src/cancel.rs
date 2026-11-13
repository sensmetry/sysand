// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Cancellation of long operations.
//!
//! A [`CancellationToken`] is observed at the *bridge checkpoints*: every
//! future run through a [`Bridge`], the solver between candidates, and a
//! git clone through `GitInterrupt`. The synchronous step in flight (an archive's
//! extraction, a lockfile's write, a `put_project` move) completes
//! atomically first; none of those goes through a bridge, and a token
//! cannot interrupt them. `sync --verify` afterwards is clean or reports
//! a state `--repair` repairs.
//!
//! Explicitly not cancellable: ad-hoc `block_on`s of a single upload,
//! which the index receives whole or not at all, and of short probes.
//!
//! A bridge that observes cancellation returns [`Cancellable::Cancelled`];
//! every other error is [`Cancellable::Inner`]. The CLI turns a cancelled
//! run into "cancelled" and exit code 130.

#[cfg(feature = "networking")]
use std::time::Duration;
use std::{future::Future, sync::Arc};

use thiserror::Error;
pub use tokio_util::sync::CancellationToken;

/// An operation stopped because its [`CancellationToken`] was cancelled.
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("cancelled")]
pub struct Cancelled;

/// A bridge's outcome: the inner operation's error, or cancellation.
#[derive(Error, Debug)]
pub enum Cancellable<E> {
    #[error(transparent)]
    Inner(E),
    #[error(transparent)]
    Cancelled(Cancelled),
}

impl<E> From<E> for Cancellable<E> {
    fn from(value: E) -> Self {
        Self::Inner(value)
    }
}

/// A current-thread runtime with every driver enabled, as the bridges
/// expect.
pub fn current_thread_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// A runtime that runs each future until it completes or the token is
/// cancelled.
#[derive(Debug, Clone)]
pub struct Bridge {
    runtime: Arc<tokio::runtime::Runtime>,
    token: CancellationToken,
}

impl Bridge {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, token: CancellationToken) -> Self {
        Self { runtime, token }
    }

    /// Run `future`; [`Cancellable::Cancelled`] when the token is cancelled
    /// first.
    pub fn run<T, E>(
        &self,
        future: impl Future<Output = Result<T, E>>,
    ) -> Result<T, Cancellable<E>> {
        self.run_value(future)
            .ok_or(Cancellable::Cancelled(Cancelled))?
            .map_err(Cancellable::Inner)
    }

    /// Run `future`; `None` when the token is cancelled first.
    pub fn run_value<T>(&self, future: impl Future<Output = T>) -> Option<T> {
        self.runtime
            .block_on(self.token.run_until_cancelled(future))
    }
}

/// The `io::Error` a cancelled synchronous reader returns. Not
/// `Interrupted`: `Read::read_to_end`, `read_exact` and `io::copy` retry
/// on that kind, and a cancelled reader must fail, not spin.
pub fn cancelled_io_error() -> std::io::Error {
    std::io::Error::other(Cancelled)
}

/// A git clone's cancellation, through gix's process-global
/// `IS_INTERRUPTED` flag.
///
/// gix consults one `AtomicBool` for every operation in the process, so
/// per-operation cancellation is only correct if one clone runs at a
/// time: `begin` takes a process-wide mutex, checks the token, and starts
/// a watcher that sets the flag when the token fires; `Drop` clears the
/// flag and stops the watcher. Two clone-bearing operations in one
/// process serialize here and cancel independently.
#[cfg(feature = "networking")]
pub struct GitInterrupt {
    _serial: std::sync::MutexGuard<'static, ()>,
    token: CancellationToken,
    stop: Arc<std::sync::atomic::AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "networking")]
static GIT_CLONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "networking")]
impl GitInterrupt {
    /// How often the watcher looks at the token.
    const POLL: Duration = Duration::from_millis(50);

    /// Wait for the process's other clones, then arm the flag for this
    /// one. `Err` when the token was cancelled before the clone began.
    pub fn begin(token: &CancellationToken) -> Result<Self, Cancelled> {
        let serial = GIT_CLONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if token.is_cancelled() {
            return Err(Cancelled);
        }
        gix::interrupt::IS_INTERRUPTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = {
            let token = token.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    if token.is_cancelled() {
                        gix::interrupt::IS_INTERRUPTED
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                    std::thread::sleep(Self::POLL);
                }
            })
        };
        Ok(Self {
            _serial: serial,
            token: token.clone(),
            stop,
            watcher: Some(watcher),
        })
    }

    /// Whether the clone this guard covers was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Whether gix's flag is currently raised.
    pub fn flag_raised() -> bool {
        gix::interrupt::IS_INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(feature = "networking")]
impl Drop for GitInterrupt {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(watcher) = self.watcher.take()
            && watcher.join().is_err()
        {
            log::debug!("the git interrupt watcher panicked");
        }
        gix::interrupt::IS_INTERRUPTED.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "./cancel_tests.rs"]
mod tests;
