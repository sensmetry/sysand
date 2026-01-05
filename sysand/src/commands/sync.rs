// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use camino::Utf8Path;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    env::local_directory::{DEFAULT_ENV_NAME, DEFAULT_MANIFEST_NAME, LocalDirectoryEnvironment},
    lock::Lock,
    project::{
        AsSyncProjectTokio, ProjectReadAsync, local_kpar::LocalKParProject,
        local_src::LocalSrcProject, memory::InMemoryProject,
        reqwest_kpar_download::ReqwestKparDownloadedProject, reqwest_src::ReqwestSrcProjectAsync,
        utils::wrapfs,
    },
};

#[allow(clippy::too_many_arguments)]
pub fn command_sync<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    update_manifest: bool,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
) -> Result<()> {
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(|src_path: &Utf8Path| LocalSrcProject {
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
        Some(|kpar_path: &Utf8Path| LocalKParProject::new_guess_root(kpar_path).unwrap()),
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
        provided_iris,
    )?;

    if update_manifest {
        let manifest = lock.to_resolved_manifest(env, &project_root)?;
        wrapfs::write(
            project_root
                .as_ref()
                .join(DEFAULT_ENV_NAME)
                .join(DEFAULT_MANIFEST_NAME),
            manifest.to_string(),
        )?;
    }

    Ok(())
}
