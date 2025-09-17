// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::PathBuf;

use anyhow::{Result, bail};
use sysand_core::{
    exclude::do_exclude,
    project::{SourceExclusionOutcome, local_src::LocalSrcProject},
};

use crate::CliError;

pub fn command_exclude(paths: Vec<String>, current_project: Option<LocalSrcProject>) -> Result<()> {
    let mut current_project =
        current_project.ok_or(CliError::MissingProject("in current directory".to_string()))?;

    for path in paths {
        let path = PathBuf::from(path);
        let unix_path = current_project.get_unix_path(&path)?;

        if let SourceExclusionOutcome {
            removed_checksum: Some(_),
            ..
        } = do_exclude(&mut current_project, unix_path)?
        {
            let excluding = "Excluded";
            let header = crate::style::CONFIG.header;
            log::info!("{header}{excluding:>12}{header:#} file: {}", path.display(),);
        } else {
            bail!("could not find {} in project metadata.", path.display())
        }
    }

    Ok(())
}
