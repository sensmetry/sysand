// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use anyhow::Result;

use crate::CliError;

pub fn command_new<P: AsRef<Path>>(
    name: Option<String>,
    version: Option<String>,
    path: P,
) -> Result<()> {
    if !path.as_ref().exists() {
        std::fs::create_dir(&path)?;
    }

    Ok(sysand_core::new::do_new(
        name.ok_or(()).or_else(|_| default_name_from_path(&path))?,
        version.unwrap_or("0.0.1".to_string()),
        &mut sysand_core::project::local_src::LocalSrcProject {
            project_path: path.as_ref().into(),
        },
    )?)
}

fn default_name_from_path<P: AsRef<Path>>(path: P) -> Result<String> {
    Ok(std::fs::canonicalize(&path)?
        .file_name()
        .ok_or(CliError::InvalidDirectory(format!(
            "Directory has no name: {}",
            path.as_ref().display()
        )))?
        .to_str()
        .ok_or(CliError::InvalidDirectory(format!(
            "Directory name is not valid Unicode: {}",
            path.as_ref().display()
        )))?
        .to_string())
}
