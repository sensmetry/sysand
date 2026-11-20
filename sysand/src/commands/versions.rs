// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand versions <iri>`: the versions a source offers, highest first.

use std::sync::Arc;

use anyhow::Result;
use fluent_uri::Iri;
use sysand_core::{
    auth::HTTPAuthentication, commands::versions::do_versions, config::Config,
    context::ProjectContext, index_location::IndexSource,
};

use crate::{commands::info::usage_resolver, get_overrides};

/// A relative override path resolves against `ctx.config_root`.
pub fn command_versions<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    config: &Config,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexSource>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: ProjectContext,
) -> Result<()> {
    let overrides = get_overrides(config, &ctx, &client, runtime.clone(), auth_policy.clone())?;
    let resolver = usage_resolver(
        overrides,
        ctx.env,
        client,
        index_urls,
        runtime,
        auth_policy,
        ctx.cancellation,
    );
    let listing = do_versions(&iri, &resolver)?;
    for ignored in &listing.ignored {
        log::warn!(
            "ignoring non-semver version `{ignored}` of `{}`",
            listing.iri
        );
    }
    for version in &listing.versions {
        println!("{version}");
    }
    Ok(())
}
