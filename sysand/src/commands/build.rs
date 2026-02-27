// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Result;
use camino::Utf8Path;
use sysand_core::{
    build::{do_build_kpar, do_build_workspace_kpars}, model::ZipCompressionMethod, project::local_src::LocalSrcProject, workspace::Workspace
};

pub fn command_build_for_project<P: AsRef<Utf8Path>>(
    path: P,
    compression: ZipCompressionMethod,
    current_project: LocalSrcProject,
) -> Result<()> {
    do_build_kpar(&current_project, &path, compression, true)?;

    Ok(())
}

pub fn command_build_for_workspace<P: AsRef<Utf8Path>>(
    path: P,
    compression: ZipCompressionMethod,
    workspace: Workspace,
) -> Result<()> {
    log::warn!(
        "Workspaces are an experimental feature\n\
        and their behavior may change even with minor\n\
        releases. For the status of this feature, see\n\
        https://github.com/sensmetry/sysand/issues/101."
    );
    do_build_workspace_kpars(&workspace, &path, compression, true)?;

    Ok(())
}
