// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Cancellation at the bridges.

use std::{sync::Arc, time::Duration};

use super::*;
use crate::{
    env::{ReadEnvironment as _, ReadEnvironmentAsync, SyncStreamIter},
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::{ProjectRead as _, ProjectReadAsync},
};

fn runtime() -> Arc<tokio::runtime::Runtime> {
    Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap(),
    )
}

/// Cancel `token` from another thread after `after`.
fn cancel_later(token: &CancellationToken, after: Duration) {
    let token = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        token.cancel();
    });
}

#[test]
fn bridge_returns_the_future_s_result() {
    let rt = runtime();
    let token = CancellationToken::new();
    let value: Result<u8, Cancellable<()>> = bridge(&rt, &token, async { Ok::<_, ()>(7) });
    assert_eq!(value.unwrap(), 7);
    let error: Result<u8, Cancellable<&str>> = bridge(&rt, &token, async { Err("boom") });
    assert!(matches!(error, Err(Cancellable::Inner("boom"))));
}

#[test]
fn bridge_is_cancelled_mid_future() {
    let rt = runtime();
    let token = CancellationToken::new();
    cancel_later(&token, Duration::from_millis(100));
    let started = std::time::Instant::now();
    let result: Result<(), Cancellable<std::io::Error>> = bridge(
        &rt,
        &token,
        std::future::pending::<Result<(), std::io::Error>>(),
    );
    assert!(matches!(result, Err(Cancellable::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(result.unwrap_err().to_string(), "cancelled");
}

/// A project whose every read never completes.
#[derive(Debug)]
struct Stalled;

impl ProjectReadAsync for Stalled {
    type Error = std::convert::Infallible;

    async fn get_project_async(
        &self,
    ) -> Result<
        (
            Option<InterchangeProjectInfoRaw>,
            Option<InterchangeProjectMetadataRaw>,
        ),
        Self::Error,
    > {
        std::future::pending().await
    }

    type SourceReader<'a>
        = futures::io::Empty
    where
        Self: 'a;

    async fn read_source_async<P: AsRef<typed_path::Utf8UnixPath>>(
        &self,
        _path: P,
    ) -> Result<Self::SourceReader<'_>, Self::Error> {
        std::future::pending().await
    }

    async fn sources_async(
        &self,
        _ctx: &crate::context::ProjectContext,
    ) -> Result<Vec<crate::lock::Source>, Self::Error> {
        std::future::pending().await
    }

    fn source_may_offer_multiple_versions(&self) -> bool {
        false
    }

    async fn checksum_canonical_variant_async(
        &self,
    ) -> Result<crate::project::ProjectChecksum, Self::Error> {
        std::future::pending().await
    }

    fn project_root(&self) -> Option<&camino::Utf8Path> {
        None
    }
}

#[test]
fn project_adapter_reports_cancelled() {
    let token = CancellationToken::new();
    let project = Stalled.to_tokio_sync_with(runtime(), token.clone());
    cancel_later(&token, Duration::from_millis(100));
    assert!(matches!(project.get_info(), Err(Cancellable::Cancelled)));
    // Once cancelled, every further bridge answers at once.
    assert!(matches!(project.get_meta(), Err(Cancellable::Cancelled)));
    assert!(matches!(
        project.checksum_canonical_hex(),
        Err(crate::project::CanonicalizationError::ProjectRead(
            Cancellable::Cancelled
        ))
    ));
    assert!(!project.is_definitely_invalid());
}

#[test]
fn reader_adapter_returns_interrupted() {
    use std::io::Read as _;
    let token = CancellationToken::new();
    token.cancel();
    let mut reader = crate::project::AsSyncReaderTokio::new(runtime(), futures::io::empty())
        .with_cancellation(token);
    let error = reader.read(&mut [0; 4]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(error.to_string(), "cancelled");
    // The kind matters: `read_to_end` retries `Interrupted` forever.
    let mut sink = Vec::new();
    reader.read_to_end(&mut sink).unwrap_err();
}

#[test]
fn stream_iter_yields_cancelled_once() {
    let token = CancellationToken::new();
    let items: Vec<Result<u8, ()>> = vec![Ok(1), Ok(2)];
    let mut iter = SyncStreamIter::new(runtime(), futures::stream::iter(items))
        .with_cancellation(token.clone());
    assert!(matches!(iter.next(), Some(Ok(1))));
    token.cancel();
    assert!(matches!(iter.next(), Some(Err(Cancellable::Cancelled))));
    assert!(iter.next().is_none());
}

#[test]
fn environment_adapter_propagates_the_token() {
    // A never-cancelled environment whose project is handed out with the
    // adapter's token: cancelling the token cancels the project too.
    #[derive(Debug)]
    struct OneProject;
    impl ReadEnvironmentAsync for OneProject {
        type ReadError = std::convert::Infallible;
        type UriStream = futures::stream::Iter<std::vec::IntoIter<Result<String, Self::ReadError>>>;
        async fn uris_async(&self) -> Result<Self::UriStream, Self::ReadError> {
            Ok(futures::stream::iter(vec![Ok("urn:one".to_owned())]))
        }
        type VersionStream = Self::UriStream;
        async fn versions_async<S: AsRef<str>>(
            &self,
            _uri: S,
        ) -> Result<Self::VersionStream, Self::ReadError> {
            Ok(futures::stream::iter(vec![Ok("1.0.0".to_owned())]))
        }
        type InterchangeProjectRead = Stalled;
        async fn get_project_async<S: AsRef<str>, T: AsRef<str>>(
            &self,
            _uri: S,
            _version: T,
        ) -> Result<Self::InterchangeProjectRead, Self::ReadError> {
            Ok(Stalled)
        }
        async fn has_version_verified_async<S: AsRef<str>, V: AsRef<str>>(
            &self,
            _uri: S,
            _version: V,
            _checksum: &crate::project::ProjectChecksum,
        ) -> Result<crate::env::ProjectChecksumResult, Self::ReadError> {
            Ok(crate::env::ProjectChecksumResult::Match)
        }
    }
    let token = CancellationToken::new();
    let env = OneProject.to_tokio_sync_with(runtime(), token.clone());
    let uris: Vec<_> = env.uris().unwrap().collect();
    assert_eq!(uris.len(), 1);
    let project = env.get_project("urn:one", "1.0.0").unwrap();
    token.cancel();
    assert!(matches!(project.get_info(), Err(Cancellable::Cancelled)));
    assert!(matches!(env.uris(), Err(Cancellable::Cancelled)));
}

#[cfg(feature = "networking")]
#[test]
fn git_interrupt_guard_serializes_and_cancels_independently() {
    let a = CancellationToken::new();
    let b = CancellationToken::new();
    {
        let guard = GitInterrupt::begin(&a).unwrap();
        // Another operation's token has no effect on this clone.
        b.cancel();
        std::thread::sleep(Duration::from_millis(150));
        assert!(!GitInterrupt::flag_raised());
        assert!(!guard.is_cancelled());
        // This clone's token raises the flag.
        a.cancel();
        std::thread::sleep(Duration::from_millis(200));
        assert!(GitInterrupt::flag_raised());
        assert!(guard.is_cancelled());
    }
    // Dropped: the flag is lowered for the next holder ...
    assert!(!GitInterrupt::flag_raised());
    // ... and a clone whose token was already cancelled never starts.
    assert!(matches!(
        GitInterrupt::begin(&b),
        Err(Cancellable::Cancelled)
    ));
    GitInterrupt::begin(&CancellationToken::new()).unwrap();
}

/// A local repository stands in for the remote.
#[cfg(all(feature = "filesystem", feature = "networking"))]
#[test]
fn git_clone_is_cancelled_before_it_starts() {
    use crate::project::gix_git_download::{
        GixDownloadedError, GixDownloadedProject, tests::git_commit_files,
    };
    let dir = camino_tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    git_commit_files(&repo, &[]).unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let project = GixDownloadedProject::new(repo.as_str())
        .unwrap()
        .with_cancellation(token);
    assert!(matches!(
        project.get_info(),
        Err(GixDownloadedError::Cancelled)
    ));
}
