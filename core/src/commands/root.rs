// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use camino::{Utf8Path, Utf8PathBuf};

use crate::{discover::discover_project, project::utils::FsIoError};

pub fn do_root<P: AsRef<Utf8Path>>(path: P) -> Result<Option<Utf8PathBuf>, Box<FsIoError>> {
    let root = discover_project(path)?.map(|e| e.root_path().clone());
    Ok(root)
}
