// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{num::NonZeroU64, sync::Arc};

use anyhow::Result;
use camino::Utf8Path;
use typed_path::Utf8UnixPathBuf;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    project::{
        AsSyncProjectTokio, KparMeta, ProjectReadAsync,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::{KparInnerPath, LocalKParProject},
        local_src::LocalSrcProject,
        reqwest_kpar_download::{
            ReqwestIndexKparDownloadedProject, ReqwestRemoteKparDownloadedProject,
        },
        reqwest_src::ReqwestSrcProjectAsync,
    },
    utils::ProvidedProjects,
    workspace::Workspace,
};

#[allow(clippy::too_many_arguments)]
pub fn command_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_usages: &ProvidedProjects,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ws: Option<&Workspace>,
) -> Result<()> {
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(
            |src_path: Utf8UnixPathBuf, checksum: String| -> LocalSrcProject {
                LocalSrcProject {
                    project_path: project_root.as_ref().join(src_path.as_str()),
                    nominal_path: Some(src_path),
                    expected_checksum: Some(checksum),
                }
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
            |kpar_path: String, kpar_size: NonZeroU64, kpar_digest: String| -> LocalKParProject {
                LocalKParProject::new(
                    project_root.as_ref().join(&kpar_path),
                    KparInnerPath::Guess,
                    Some(kpar_path.into()),
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
    )?;

    env.merge_lock(lock, ws);
    env.write()?;

    Ok(())
}
