// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::HashMap, path::PathBuf};

use anyhow::Result;
use url::ParseError;

use sysand_core::{
    env::local_directory::LocalDirectoryEnvironment,
    lock::Lock,
    project::{
        local_kpar::LocalKParProject, local_src::LocalSrcProject, memory::InMemoryProject,
        reqwest_kpar_download::ReqwestKparDownloadedProject, reqwest_src::ReqwestSrcProject,
    },
};

pub fn command_sync(
    lock: Lock,
    project_root: PathBuf,
    env: &mut LocalDirectoryEnvironment,
    client: reqwest::blocking::Client,
    provided_iris: &HashMap<String, Vec<InMemoryProject>>,
) -> Result<()> {
    sysand_core::commands::sync::do_sync(
        lock,
        env,
        Some(|src_path: String| LocalSrcProject {
            project_path: project_root.join(src_path),
        }),
        Some(
            |remote_src: String| -> Result<ReqwestSrcProject, ParseError> {
                Ok(ReqwestSrcProject {
                    client: client.clone(),
                    url: reqwest::Url::parse(&remote_src)?,
                })
            },
        ),
        // TODO: Fix error handling here
        Some(|kpar_path: String| LocalKParProject::new_guess_root(kpar_path).unwrap()),
        Some(
            |remote_kpar: String| -> Result<ReqwestKparDownloadedProject, ParseError> {
                Ok(
                    ReqwestKparDownloadedProject::new_guess_root(reqwest::Url::parse(
                        &remote_kpar,
                    )?)
                    .unwrap(),
                )
            },
        ),
        provided_iris,
    )?;
    Ok(())
}
