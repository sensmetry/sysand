// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use anyhow::{Result, anyhow};

use camino::Utf8Path;
use sysand_core::{root::do_root, workspace::model_toml::StartRegion};

use crate::CliError;

/// Prints the workspace root, from the root and from a member alike.
///
/// The exception is a project under an `exclude`d directory. It is not
/// in the workspace, so what it is inside of is itself, and printing the
/// root of a workspace it is not part of would be wrong.
///
/// A project under the root that is neither a member nor excluded is a
/// hard error, and is not handled here.
pub fn command_print_root(
    path: &Utf8Path,
    workspace: Option<(&Utf8Path, &StartRegion)>,
) -> Result<()> {
    if let Some((root, start)) = workspace {
        let root = match start {
            StartRegion::Standalone(project) => project.as_path(),
            StartRegion::Member(_) | StartRegion::Workspace => root,
        };
        crate::output::document("root", root.as_str());
        crate::output::text(|| println!("{root}"));
        return Ok(());
    }
    match do_root(path)? {
        Some(root) => {
            crate::output::document("root", root.as_str());
            crate::output::text(|| println!("{root}"));
            Ok(())
        }
        None => Err(anyhow!(CliError::InvalidDirectory(
            "not inside a project".to_owned(),
        ))),
    }
}
