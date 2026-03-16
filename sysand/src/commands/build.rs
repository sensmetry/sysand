// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::{Result, bail};
use camino::Utf8Path;
use sysand_core::{
    build::{KParBuildError, KparCompressionMethod, do_build_kpar, do_build_workspace_kpars},
    project::local_src::LocalSrcProject,
    workspace::Workspace,
};

pub fn command_build_for_project<P: AsRef<Utf8Path>>(
    path: P,
    compression: KparCompressionMethod,
    current_project: LocalSrcProject,
    allow_path_usage: bool,
    readme_source_path: Option<&Utf8Path>,
) -> Result<()> {
    match do_build_kpar(
        &current_project,
        &path,
        compression,
        true,
        allow_path_usage,
        readme_source_path,
    ) {
        Ok(_) => Ok(()),
        Err(err) => match err {
            KParBuildError::PathUsage(_) => bail!(
                "{err}\n\
                to build anyway, pass `--allow-path-usage`"
            ),
            _ => bail!(err),
        },
    }
}

pub fn command_build_for_workspace<P: AsRef<Utf8Path>>(
    path: P,
    compression: KparCompressionMethod,
    workspace: Workspace,
    allow_path_usage: bool,
) -> Result<()> {
    log::warn!(
        "Workspaces are an experimental feature\n\
        and their behavior may change even with minor\n\
        releases. For the status of this feature, see\n\
        https://github.com/sensmetry/sysand/issues/101."
    );
    do_build_workspace_kpars(&workspace, &path, compression, true, allow_path_usage)?;

    Ok(())
}
