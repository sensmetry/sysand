// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use crate::CliError;
use anyhow::Result;
use sysand_core::{new::NewError, project::utils::wrapfs};

pub fn command_new<P: AsRef<Path>>(
    name: Option<String>,
    version: Option<String>,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    path: P,
) -> Result<()> {
    if !path.as_ref().exists() {
        wrapfs::create_dir(&path)?;
    }
    let version = version.unwrap_or_else(|| "0.0.1".to_string());

    (match sysand_core::new::do_new(
        name.ok_or(()).or_else(|_| default_name_from_path(&path))?,
        version,
        license,
        &mut sysand_core::project::local_src::LocalSrcProject {
            project_path: path.as_ref().into(),
        },
    ) {
        // TODO: don't simply eat the error, actually create the project
        Ok(_) => Ok(()),
        Err(err) => match err {
            NewError::SemVerParse(..) => {
                if no_semver {
                    Ok(())
                } else {
                    Err(err)
                }
            }
            NewError::SPDXLicenseParse(..) => {
                if no_spdx {
                    Ok(())
                } else {
                    Err(err)
                }
            }
            _ => Err(err),
        },
    })?;
    Ok(())
}

fn default_name_from_path<P: AsRef<Path>>(path: P) -> Result<String> {
    Ok(wrapfs::canonicalize(&path)?
        .file_name()
        .ok_or(CliError::InvalidDirectory(format!(
            "Directory `{}` has no name",
            path.as_ref().display()
        )))?
        .to_str()
        .ok_or(CliError::InvalidDirectory(format!(
            "Directory name `{}` is not valid Unicode",
            path.as_ref().display()
        )))?
        .to_string())
}
