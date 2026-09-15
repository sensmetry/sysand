// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

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
    update_index: bool,
    allow_path_usage: bool,
) -> Result<()> {
    match do_build_kpar(
        &current_project,
        &path,
        compression,
        update_index,
        allow_path_usage,
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
    update_index: bool,
    allow_path_usage: bool,
) -> Result<()> {
    log::warn!(
        "Workspaces are an experimental feature\n\
        and their behavior may change even with minor\n\
        releases. For the status of this feature, see\n\
        https://github.com/sensmetry/sysand/issues/101."
    );
    if let Err(failure) = do_build_workspace_kpars(
        &workspace,
        &path,
        compression,
        update_index,
        allow_path_usage,
    ) {
        // A member's build error carries no member identity -- two
        // members with no manifest produce the same sentence, and the
        // failure arrives before the `Building kpar` line that would have
        // named a path -- so the member is named here or nowhere.
        for failed in &failure.failed {
            log::error!(
                "failed to build member `{}`: {}",
                failed.member,
                failed.error
            );
        }
        return Err(failure.into());
    }

    Ok(())
}
