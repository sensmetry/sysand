// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, bail};
use camino::Utf8Path;

use crate::CliError;
use sysand_core::{
    build::{KParBuildError, KparCompressionMethod, do_build_kpar_with, do_build_workspace_kpars},
    context::ProjectContext,
    project::{ProjectDefinition, local_src::LocalSrcError},
    utils::format_err,
    workspace::model_toml::Workspace,
};

/// The manifest of the context's current project, for a build.
pub fn current_definition_for_build(
    ctx: &ProjectContext,
) -> Result<ProjectDefinition, KParBuildError<LocalSrcError>> {
    ctx.current_definition()
        .map_err(KParBuildError::ProjectRead)?
        .ok_or(KParBuildError::MissingInfo)
}

pub fn command_build_for_project<P: AsRef<Utf8Path>>(
    path: P,
    compression: KparCompressionMethod,
    ctx: &ProjectContext,
    update_index: bool,
    allow_path_usage: bool,
) -> Result<()> {
    let current_project = ctx
        .current_project
        .as_ref()
        .ok_or(CliError::MissingProjectCurrentDir)?;
    match do_build_kpar_with(
        current_project,
        || ctx.current_definition(),
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
    workspace: &Workspace,
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
        workspace,
        &path,
        compression,
        update_index,
        allow_path_usage,
    ) {
        // A member's build error carries no member identity (two members
        // with no manifest produce the same sentence, and the failure
        // arrives before the `Building kpar` line that would name a path),
        // so the member is named here.
        for failed in &failure.failed {
            log::error!(
                "failed to build member `{}`: {}",
                failed.member,
                format_err(&*failed.error)
            );
        }
        return Err(failure.into());
    }

    Ok(())
}
