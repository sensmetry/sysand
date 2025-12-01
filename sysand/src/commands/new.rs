// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use crate::CliError;
use anyhow::Result;
use sysand_core::project::utils::wrapfs;

pub fn command_new<P: AsRef<Path>>(
    name: Option<String>,
    version: Option<String>,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    path: P,
) -> Result<()> {
    if !path.as_ref().exists() {
        wrapfs::create_dir_all(&path)?;
    }
    let version = version.unwrap_or_else(|| "0.0.1".to_string());
    let name = match name {
        Some(n) => n,
        None => default_name_from_path(&path)?,
    };

    sysand_core::new::do_new_ext(
        name,
        version,
        no_semver,
        license,
        no_spdx,
        &mut sysand_core::project::local_src::LocalSrcProject {
            project_path: path.as_ref().into(),
        },
    )?;
    Ok(())
}

fn default_name_from_path<P: AsRef<Path>>(path: P) -> Result<String> {
    Ok(wrapfs::canonicalize(&path)?
        .file_name()
        .ok_or(CliError::InvalidDirectory(format!(
            "path `{}` is not a directory",
            path.as_ref().display()
        )))?
        .to_str()
        .ok_or(CliError::InvalidDirectory(format!(
            "directory name `{:?}` is not valid Unicode",
            path.as_ref()
        )))?
        .to_string())
}
