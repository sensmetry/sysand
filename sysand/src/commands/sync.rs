// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{num::NonZeroU64, sync::Arc};

use camino::Utf8Path;
use thiserror::Error;
use typed_path::Utf8UnixPathBuf;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    commands::sync::{SyncError, SyncOutcome},
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    project::{
        AsSyncProjectTokio, KparMeta, ProjectReadAsync as _,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        reqwest_kpar_download::{
            ReqwestIndexKparDownloadedProject, ReqwestRemoteKparDownloadedProject,
        },
        reqwest_src::ReqwestSrcProjectAsync,
        utils::FsIoError,
    },
    utils::ProvidedProjects,
};

/// The `SyncError` instantiation `command_sync` fails with when the sync
/// itself fails.
pub type CliSyncError = SyncError<ParseError, GixDownloadedError, LocalDirectoryEnvironment>;

/// Why `command_sync` failed.
#[derive(Debug, Error)]
pub enum CommandSyncError {
    /// The sync itself, see `SyncError`.
    #[error(transparent)]
    Sync(#[from] CliSyncError),
    /// The sync itself, where the remedy names a flag only the CLI has.
    /// The hint is part of the message so that every printer of this
    /// error carries it.
    #[error("{0}\n       (hint: pass `--include-std` to fetch it from its IRI)")]
    IncludeStd(CliSyncError),
    /// Writing the environment metadata after a completed sync.
    #[error(transparent)]
    WriteMetadata(#[from] Box<FsIoError>),
}

/// Install the lockfile into `env`. `outcome` is filled in progressively,
/// so on `Err` it holds what was installed and pruned before the failure.
pub fn command_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_usages: &ProvidedProjects,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    no_prune: bool,
    outcome: &mut SyncOutcome,
) -> Result<(), CommandSyncError> {
    // `project_root` is the root the lockfile's `src_path`s are relative
    // to: the workspace root for a workspace's lockfile.
    let relative_root = project_root.as_ref();
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(
            |src_path: Utf8UnixPathBuf, checksum: String| -> LocalSrcProject {
                LocalSrcProject::new_access_with_checksum(
                    relative_root.join(src_path.as_str()),
                    Some(src_path),
                    checksum,
                )
            },
        ),
        Some(
            |remote_src: String,
             checksum: String|
             -> Result<AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>, ParseError> {
                Ok(ReqwestSrcProjectAsync {
                    client: client.clone(),
                    url: url::Url::parse(&remote_src)?,
                    auth_policy: auth_policy.clone(),
                    expected_checksum: Some(checksum),
                }
                .to_tokio_sync(runtime.clone()))
            },
        ),
        Some(
            |kpar_path: Utf8UnixPathBuf,
             kpar_size: NonZeroU64,
             kpar_digest: String|
             -> LocalKParProject {
                LocalKParProject::new_with_expected(
                    relative_root.join(kpar_path.as_str()),
                    KparInnerPath::Guess,
                    Some(kpar_path),
                    KparMeta {
                        size_bytes: kpar_size,
                        sha256_hex: kpar_digest,
                    },
                )
            },
        ),
        // TODO: Fix error handling here
        Some(
            |index_kpar: String,
             index_kpar_size: NonZeroU64,
             index_kpar_digest: String|
             -> Result<
                AsSyncProjectTokio<ReqwestRemoteKparDownloadedProject<Policy>>,
                ParseError,
            > {
                let project = ReqwestRemoteKparDownloadedProject::new_guess_root(
                    url::Url::parse(&index_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    Some(KparMeta {
                        size_bytes: index_kpar_size,
                        sha256_hex: index_kpar_digest,
                    }),
                )
                .unwrap();
                Ok(project.to_tokio_sync(runtime.clone()))
            },
        ),
        Some(
            |index_kpar: String,
             index_kpar_size: NonZeroU64,
             index_kpar_digest: String|
             -> Result<
                AsSyncProjectTokio<ReqwestIndexKparDownloadedProject<Policy>>,
                ParseError,
            > {
                let project = ReqwestIndexKparDownloadedProject::new(
                    url::Url::parse(&index_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    index_kpar_size,
                    index_kpar_digest,
                )
                .unwrap();
                Ok(project.to_tokio_sync(runtime.clone()))
            },
        ),
        Some(
            |remote_git: String| -> Result<GixDownloadedProject, GixDownloadedError> {
                GixDownloadedProject::new(remote_git)
            },
        ),
        provided_usages,
        no_prune,
        outcome,
    )
    // Appendix A2 row 39's second half. `plan-phase1.md`: "where a hint
    // names a command, the CLI appends it" -- the bindings reach the
    // same error and have no `--include-std` to pass.
    .map_err(|error| match &error {
        SyncError::UnknownStdLib { .. } | SyncError::InvalidProvidedVersion { .. } => {
            CommandSyncError::IncludeStd(error)
        }
        _ => CommandSyncError::Sync(error),
    })?;

    env.merge_lock(lock);
    env.write()?;

    Ok(())
}
