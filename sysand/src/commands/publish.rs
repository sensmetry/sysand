// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use anyhow::Result;
use camino::Utf8Path;
use sysand_core::{auth::HTTPAuthentication, commands::publish::do_publish_kpar};

pub fn command_publish<P: AsRef<Utf8Path>, Policy: HTTPAuthentication>(
    kpar_path: P,
    index_url: &str,
    auth_policy: Arc<Policy>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let response = do_publish_kpar(kpar_path, index_url, auth_policy, client, runtime)?;

    let header = sysand_core::style::get_style_config().header;
    let published = "Published";
    if response.is_new_project {
        log::info!("{header}{published:>12}{header:#} new project successfully");
    } else {
        log::info!("{header}{published:>12}{header:#} new release successfully");
    }

    Ok(())
}
