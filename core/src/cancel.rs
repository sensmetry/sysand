// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Cancellation of long operations.
//!
//! A [`CancellationToken`] is observed at the *bridge checkpoints*: every
//! `block_on` of the `AsSync*Tokio` adapters and of `SyncStreamIter`, the
//! solver between candidates, and a git clone through
//! [`GitInterrupt`]. The synchronous step in flight -- an archive's
//! extraction, a lockfile's write, a `put_project` move -- completes
//! atomically first; none of those goes through a bridge, and a token
//! cannot interrupt them. `sync --verify` afterwards is clean or reports
//! a state `--repair` repairs.
//!
//! Explicitly not cancellable: the ad-hoc `block_on`s in
//! `commands::publish` (one upload, which the index receives whole or
//! not at all) and `commands::auth` (short probes).
//!
//! A bridge that observes cancellation returns [`Cancellable::Cancelled`];
//! every other error is [`Cancellable::Inner`]. The CLI turns a cancelled
//! run into "cancelled" and exit code 130.

use std::future::Future;
#[cfg(feature = "networking")]
use std::{sync::Arc, time::Duration};

use thiserror::Error;
pub use tokio_util::sync::CancellationToken;

/// A bridge's outcome: the inner operation's error, or cancellation.
#[derive(Error, Debug)]
pub enum Cancellable<E> {
    #[error(transparent)]
    Inner(E),
    #[error("cancelled")]
    Cancelled,
}

impl<E> Cancellable<E> {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Map the inner error, keeping cancellation.
    pub fn map_inner<F, G: FnOnce(E) -> F>(self, f: G) -> Cancellable<F> {
        match self {
            Self::Inner(e) => Cancellable::Inner(f(e)),
            Self::Cancelled => Cancellable::Cancelled,
        }
    }
}

impl<E> From<E> for Cancellable<E> {
    fn from(value: E) -> Self {
        Self::Inner(value)
    }
}

/// Run `future` on `runtime` until it completes or `token` is cancelled.
pub fn bridge<T, E, F: Future<Output = Result<T, E>>>(
    runtime: &tokio::runtime::Runtime,
    token: &CancellationToken,
    future: F,
) -> Result<T, Cancellable<E>> {
    match runtime.block_on(token.run_until_cancelled(future)) {
        Some(Ok(value)) => Ok(value),
        Some(Err(error)) => Err(Cancellable::Inner(error)),
        None => Err(Cancellable::Cancelled),
    }
}

/// [`bridge`] for a future whose output is not a `Result`: `None` when
/// cancelled.
pub fn bridge_value<T, F: Future<Output = T>>(
    runtime: &tokio::runtime::Runtime,
    token: &CancellationToken,
    future: F,
) -> Option<T> {
    runtime.block_on(token.run_until_cancelled(future))
}

/// The `io::Error` a cancelled synchronous reader returns. Not
/// `Interrupted`: `Read::read_to_end`, `read_exact` and `io::copy` retry
/// on that kind, and a cancelled reader must fail, not spin.
pub fn cancelled_io_error() -> std::io::Error {
    std::io::Error::other("cancelled")
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
    pub fn begin(token: &CancellationToken) -> Result<Self, Cancellable<std::convert::Infallible>> {
        let serial = GIT_CLONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if token.is_cancelled() {
            return Err(Cancellable::Cancelled);
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
