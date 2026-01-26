// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use url::ParseError;

use sysand_core::{
    auth::HTTPAuthentication,
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    project::{
        AsSyncProjectTokio, ProjectReadAsync, local_kpar::LocalKParProject,
        local_src::LocalSrcProject, memory::InMemoryProject,
        reqwest_kpar_download::ReqwestKparDownloadedProject, reqwest_src::ReqwestSrcProjectAsync,
    },
};

pub fn command_sync<P: AsRef<Path>, Pol: HTTPAuthentication>(
    lock: &Lock,
    project_root: P,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest_middleware::ClientWithMiddleware,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Pol>,
) -> Result<()> {
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(|src_path: String| LocalSrcProject {
            project_path: project_root.as_ref().join(src_path),
        }),
        Some(
            |remote_src: String| -> Result<AsSyncProjectTokio<ReqwestSrcProjectAsync<Pol>>, ParseError> {
                Ok(ReqwestSrcProjectAsync {
                    client: client.clone(),
                    url: reqwest::Url::parse(&remote_src)?,
                    auth_policy: auth_policy.clone()
                }
                .to_tokio_sync(runtime.clone()))
            },
        ),
        // TODO: Fix error handling here
        Some(|kpar_path: String| LocalKParProject::new_guess_root(kpar_path).unwrap()),
        Some(
            |remote_kpar: String| -> Result<AsSyncProjectTokio<ReqwestKparDownloadedProject<Pol>>, ParseError> {
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
    Ok(())
}
