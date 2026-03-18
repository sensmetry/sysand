// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::CliError;
use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use sysand_core::project::{local_src::LocalSrcProject, utils::wrapfs};

const DEFAULT_VERSION: &str = "0.0.1";

pub fn command_init(
    name: Option<String>,
    publisher: Option<String>,
    version: Option<String>,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    path: Option<String>,
) -> Result<()> {
    let path = match path {
        Some(p) => {
            wrapfs::create_dir_all(&p)?;

            p.into()
        }
        None => Utf8PathBuf::from("."),
    };
    let version = version.unwrap_or_else(|| DEFAULT_VERSION.to_owned());
    let name = match name {
        Some(n) => n,
        None => default_name_from_path(&path)?,
    };

    sysand_core::init::do_init_ext(
        name,
        publisher,
        version,
        no_semver,
        license,
        no_spdx,
        &mut LocalSrcProject {
            nominal_path: None,
            project_path: path,
        },
    )?;
    Ok(())
}

fn default_name_from_path<P: AsRef<Utf8Path>>(path: P) -> Result<String> {
    Ok(wrapfs::canonicalize(&path)?
        .file_name()
        .ok_or_else(|| {
            CliError::InvalidDirectory(format!("path `{}` is not a directory", path.as_ref()))
        })?
        .to_string())
}
