// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use camino::Utf8Path;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    context::ProjectContext,
    env::local_directory::{LocalDirectoryEnvironment, metadata::load_env_metadata},
    lock::Lock,
    project::{
        AsSyncProjectTokio, ProjectReadAsync,
        gix_git_download::{GixDownloadedError, GixDownloadedProject},
        local_kpar::LocalKParProject,
        local_src::LocalSrcProject,
        memory::InMemoryProject,
        reqwest_kpar_download::ReqwestKparDownloadedProject,
        reqwest_src::ReqwestSrcProjectAsync,
        utils::wrapfs,
    },
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
    ctx: &ProjectContext,
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
            |remote_kpar: String| -> Result<AsSyncProjectTokio<ReqwestKparDownloadedProject<Policy>>, ParseError> {
                Ok(
                    ReqwestKparDownloadedProject::new_guess_root(reqwest::Url::parse(
                        &remote_kpar,
                    )?, client.clone(), auth_policy.clone())
                    .unwrap().to_tokio_sync(runtime.clone()),
                )
            },
        ),
        Some(|remote_git: String| -> Result<GixDownloadedProject, GixDownloadedError> {
            GixDownloadedProject::new(remote_git)
        }),
        provided_iris,
    )?;

    // TODO: Integrate the updating of metadata into `LocalDirectoryEnvironment` itself.
    //       This will likely require updating the `WriteEnvironment` trait to support
    //       multiple identifiers per project.
    let lock_metadata = lock.to_env_metadata(env, ctx)?;
    let env_metadata = if wrapfs::is_file(env.metadata_path())? {
        let mut env_metadata = load_env_metadata(env.metadata_path())?;
        env_metadata.merge(lock_metadata);
        env_metadata
    } else {
        lock_metadata
    };

    wrapfs::write(env.metadata_path(), env_metadata.to_string())?;

    Ok(())
}
