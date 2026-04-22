// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{collections::HashMap, num::NonZeroU64, sync::Arc};

use anyhow::Result;
use camino::Utf8Path;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    project::{
        AsSyncProjectTokio, ProjectReadAsync,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
        memory::InMemoryProject,
        reqwest_kpar_download::ReqwestKparDownloadedProject,
        reqwest_src::ReqwestSrcProjectAsync,
    },
    workspace::Workspace,
};

#[allow(clippy::too_many_arguments)]
pub fn command_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ws: Option<&Workspace>,
) -> Result<()> {
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(|src_path: &Utf8Path| LocalSrcProject {
            nominal_path: Some(src_path.to_path_buf()),
            project_path: project_root.as_ref().join(src_path),
        }),
        Some(
            |remote_src: String| -> Result<AsSyncProjectTokio<ReqwestSrcProjectAsync<Policy>>, ParseError> {
                Ok(ReqwestSrcProjectAsync {
                    client: client.clone(),
                    url: reqwest::Url::parse(&remote_src)?,
                    auth_policy: auth_policy.clone()
                }
                .to_tokio_sync(runtime.clone()))
            },
        ),
        // TODO: Fix error handling here
        Some(|kpar_path: &Utf8Path| LocalKParProject::new_guess_root_nominal(project_root.as_ref().join(kpar_path), kpar_path).unwrap()),
        Some(
            |remote_kpar: String|
             -> Result<AsSyncProjectTokio<ReqwestKparDownloadedProject<Policy>>, ParseError> {
                let project = ReqwestKparDownloadedProject::new_guess_root(
                    reqwest::Url::parse(&remote_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    None, None
                )
                .unwrap();
                Ok(project.to_tokio_sync(runtime.clone()))
            },
        ),
        Some(
            |index_kpar: String,
             index_kpar_size: NonZeroU64,
             index_kpar_digest: String|
             -> Result<AsSyncProjectTokio<ReqwestKparDownloadedProject<Policy>>, ParseError> {
                let project = ReqwestKparDownloadedProject::new_guess_root(
                    reqwest::Url::parse(&index_kpar)?,
                    client.clone(),
                    auth_policy.clone(),
                    Some(index_kpar_digest), Some(index_kpar_size)
                )
                .unwrap();
                Ok(project.to_tokio_sync(runtime.clone()))
            },
        ),
        Some(|remote_git: String| -> Result<GixDownloadedProject, GixDownloadedError> {
            GixDownloadedProject::new(remote_git)
        }),
        provided_iris,
    )?;

    env.merge_lock(lock, ws);
    env.write()?;

    Ok(())
}
