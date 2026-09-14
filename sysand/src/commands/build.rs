// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use camino::Utf8Path;
use sysand_core::{
    build::{KParBuildError, KparCompressionMethod, do_build_kpar, do_build_workspace_kpars},
    project::local_src::{LocalSrcProject, ProjectFormat, detect_format},
    workspace::Workspace,
};

pub fn command_build_for_project<P: AsRef<Utf8Path>>(
    path: P,
    compression: KparCompressionMethod,
    current_project: LocalSrcProject,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<()> {
    // A view of a `model.toml` project is not what a build needs:
    // `created` is a placeholder and the local usages are not rewritten
    // into the PURLs an archive carries. Refusing is better than writing
    // an archive that looks right and is not.
    if detect_format(current_project.root_path())? == ProjectFormat::Toml {
        bail!(
            "building a `model.toml` project is not supported yet;\n\
            {SP:>8} this version of Sysand builds KPARs from `.project.json` projects only",
            SP = ' '
        );
    }
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
    do_build_workspace_kpars(
        &workspace,
        &path,
        compression,
        update_index,
        allow_path_usage,
    )?;

    Ok(())
}
