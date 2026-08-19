// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use crate::CliError;
use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use sysand_core::{
    context::ProjectContext,
    discover::{discover_project, discover_workspace},
    project::{local_src::LocalSrcProject, utils::wrapfs},
    utils::{RelativePathKind, parse_relative_unix_path},
    workspace::Workspace,
};

const DEFAULT_VERSION: &str = "0.0.1";

pub fn command_init(
    name: Option<String>,
    publisher: String,
    version: Option<String>,
    no_semver: bool,
    license: Option<String>,
    no_spdx: bool,
    path: Option<String>,
    ctx: ProjectContext,
) -> Result<()> {
    let target = match path {
        Some(p) => {
            wrapfs::create_dir_all(&p)?;
            p.into()
        }
        None => Utf8PathBuf::from("."),
    };
    warn_parent_project_workspace(&target, &ctx)?;

    let version = version.unwrap_or_else(|| DEFAULT_VERSION.to_owned());
    let name = match name {
        Some(n) => n,
        None => default_name_from_path(&target)?,
    };

    sysand_core::init::do_init_ext(
        name,
        publisher,
        version,
        no_semver,
        license,
        no_spdx,
        &mut LocalSrcProject::new_access(target, None),
    )?;
    Ok(())
}

fn default_name_from_path<P: AsRef<Utf8Path>>(path: P) -> Result<String> {
    Ok(wrapfs::canonicalize(&path)?
        .file_name()
        .ok_or_else(|| {
            CliError::InvalidDirectory(format!("path `{}` is not a directory", path.as_ref()))
        })?
        .to_owned())
}

pub fn warn_parent_project_workspace(target: &Utf8Path, ctx: &ProjectContext) -> Result<()> {
    if parse_relative_unix_path(target.as_str(), RelativePathKind::SubDirectory).is_ok() {
        // Target is a subdirectory of cwd
        if let Some(existing_project) = &ctx.current_project {
            warn_parent_project(existing_project);
        }
        if let Some(existing_workspace) = &ctx.current_workspace {
            warn_parent_workspace(existing_workspace);
        }
    } else {
        // Target may be out of cwd, so cwd-derived ctx is irrelevant
        if let Some(existing_project) = discover_project(target)? {
            warn_parent_project(&existing_project);
        }
        if let Some(existing_workspace) = discover_workspace(target)? {
            warn_parent_workspace(&existing_workspace);
        }
    }
    Ok(())
}

fn warn_parent_project(existing_project: &LocalSrcProject) {
    log::warn!(
        "found an existing project in one of target path's parent\n\
        {:>8} directories `{}`",
        ' ',
        existing_project.root_path()
    );
}

fn warn_parent_workspace(existing_workspace: &Workspace) {
    log::warn!(
        "found an existing workspace in one of target path's parent\n\
        {:>8} directories `{}`;\n\
        you may want to add this project to the workspace",
        ' ',
        existing_workspace.root_path()
    );
}
