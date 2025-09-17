// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::env::WriteEnvironment;

pub fn do_env_uninstall<S: AsRef<str>, E: WriteEnvironment>(
    uri: S,
    version: Option<S>,
    mut env: E,
) -> Result<(), E::WriteError> {
    let uninstalling = "Uninstalling";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{uninstalling:>12}{header:#} {}", uri.as_ref());

    if let Some(version) = version {
        env.del_project_version(uri, version)?;
    } else {
        env.del_uri(uri)?;
    }
    Ok(())
}
