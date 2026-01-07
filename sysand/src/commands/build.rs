// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8Path;
use sysand_core::{
    build::{do_build_kpar, do_build_workspace_kpars},
    project::local_src::LocalSrcProject,
    workspace::Workspace,
};

pub fn command_build_for_project<P: AsRef<Utf8Path>>(
    path: P,
    current_project: LocalSrcProject,
    allow_path_usage: bool,
) -> Result<()> {
    do_build_kpar(&current_project, &path, true, allow_path_usage)?;

    Ok(())
}

pub fn command_build_for_workspace<P: AsRef<Utf8Path>>(
    path: P,
    workspace: Workspace,
    allow_path_usage: bool,
) -> Result<()> {
    log::warn!(
        "Workspaces are an experimental feature\n\
        and their behavior may change even with minor\n\
        releases. For the status of this feature, see\n\
        https://github.com/sensmetry/sysand/issues/101."
    );
    do_build_workspace_kpars(&workspace, &path, true, allow_path_usage)?;

    Ok(())
}
