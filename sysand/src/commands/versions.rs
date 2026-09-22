// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! `sysand versions <iri>`: the versions a source offers, highest first.

use std::sync::Arc;

use anyhow::Result;
use fluent_uri::Iri;
use sysand_core::{
    auth::HTTPAuthentication, commands::versions::do_versions, context::ProjectContext,
    index_location::IndexSource, resolve::standard::standard_resolver,
};

use crate::output;

pub fn command_versions<Policy: HTTPAuthentication>(
    iri: Iri<String>,
    client: reqwest_middleware::ClientWithMiddleware,
    index_urls: Option<Vec<IndexSource>>,
    runtime: Arc<tokio::runtime::Runtime>,
    auth_policy: Arc<Policy>,
    ctx: ProjectContext,
) -> Result<()> {
    let resolver = standard_resolver(
        ctx.env,
        Some(client),
        index_urls,
        runtime,
        auth_policy,
        ctx.cancellation,
    )?;
    let listing = do_versions(&iri, &resolver)?;
    let versions: Vec<String> = listing.versions.iter().map(ToString::to_string).collect();
    for ignored in &listing.ignored {
        log::warn!(
            "ignoring non-semver version `{ignored}` of `{}`",
            listing.iri
        );
    }
    output::document("iri", listing.iri.as_str());
    output::document("versions", versions.clone());
    output::document("ignored", listing.ignored);
    output::text(|| {
        for version in &versions {
            println!("{version}");
        }
    });
    Ok(())
}
