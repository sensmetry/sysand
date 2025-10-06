// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use anyhow::Result;
use sysand_core::{build::do_build_kpar, project::local_src::LocalSrcProject};

pub fn command_build<P: AsRef<Path>>(path: P, current_project: LocalSrcProject) -> Result<()> {
    do_build_kpar(&current_project, &path, true)?;

    Ok(())
}
