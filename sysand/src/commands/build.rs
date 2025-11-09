// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use anyhow::Result;
use sysand_core::{
    build::{do_build_kpar, do_build_workspace_kpars},
    project::local_src::LocalSrcProject,
    workspace::Workspace,
};

pub fn command_build_for_project<P: AsRef<Path>>(
    path: P,
    current_project: LocalSrcProject,
) -> Result<()> {
    do_build_kpar(&current_project, &path, true)?;

    Ok(())
}

pub fn command_build_for_workspace<P: AsRef<Path>>(path: P, workspace: Workspace) -> Result<()> {
    log::warn!(
        "Workspaces are an experimental feature and their behavior may change even with minor \
        releases. For the status of this feature, see \
        https://github.com/sensmetry/sysand/issues/101."
    );
    do_build_workspace_kpars(&workspace, &path, true)?;

    Ok(())
}
