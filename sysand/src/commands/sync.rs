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
    workspace::Workspace,
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
    ws: Option<&Workspace>,
    no_prune: bool,
    outcome: &mut SyncOutcome,
) -> Result<(), CommandSyncError> {
    #[expect(clippy::or_fun_call, reason = "cheap")]
    let relative_root = ws.map_or(project_root.as_ref(), Workspace::root_path);
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(
            |src_path: Utf8UnixPathBuf,
             publisher: Option<String>,
             name: String,
             checksum: String|
             -> LocalSrcProject {
                LocalSrcProject::new_for_sync(
                    relative_root.join(src_path.as_str()),
                    Some(src_path),
                    publisher,
                    name,
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
                    url: reqwest::Url::parse(&remote_src)?,
                    auth_policy: auth_policy.clone(),
                    expected_checksum: Some(checksum),
                }
                .to_tokio_sync(runtime.clone()))
            },
        ),
        Some(
            |kpar_path: Utf8UnixPathBuf,
             kpar_size: NonZeroU64,
             kpar_digest: String,
             publisher: Option<String>,
             name: String|
             -> LocalKParProject {
                LocalKParProject::new_for_sync(
                    relative_root.join(kpar_path.as_str()),
                    KparInnerPath::Guess,
                    Some(kpar_path),
                    publisher,
                    name,
                    Some(KparMeta {
                        size_bytes: kpar_size,
                        sha256_hex: kpar_digest,
                    }),
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
                    reqwest::Url::parse(&index_kpar)?,
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
                    reqwest::Url::parse(&index_kpar)?,
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
    )?;

    env.merge_lock(lock, ws);
    env.write()?;

    Ok(())
}
