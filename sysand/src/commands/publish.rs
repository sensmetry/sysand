// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{
    auth::{GlobMapResult, StandardHTTPAuthentication},
    build::default_kpar_path,
    commands::publish::{
        EndpointKind, build_upload_url, do_publish, prepare_publish_payload,
        validate_endpoint_url_shape,
    },
    context::ProjectContext,
    env::discovery::fetch_well_known,
    project::utils::wrapfs,
};
use url::Url;

use crate::CliError;

pub fn command_publish(
    path: Option<Utf8PathBuf>,
    index: Url,
    ctx: &ProjectContext,
    auth_policy: Arc<StandardHTTPAuthentication>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let kpar_path = resolve_publish_kpar_path(path, ctx)?;
    if !wrapfs::is_file(&kpar_path)? {
        bail!("KPAR file not found at `{kpar_path}`, run `sysand build` first");
    }
    // Reject obviously-malformed discovery-root URLs (bad scheme,
    // query/fragment components) before issuing any network request —
    // a config typo should not cost a DNS lookup + connect attempt.
    validate_endpoint_url_shape(&index, EndpointKind::DiscoveryRoot)?;
    // Validate and prepare the kpar payload before any network work,
    // so that kpar-content errors (bad semver, invalid publisher/name,
    // oversized archive) surface before discovery or credential
    // matching does.
    let prepared = prepare_publish_payload(&kpar_path)?;

    // Well-known discovery: the user-configured URL is the discovery
    // root. Resolve `api_root` via `.well-known/sysand-index.json`
    // before composing the upload URL, so that credential matching
    // (below) and the eventual POST in `do_publish` share the same
    // API-side URL. `fetch_well_known` follows redirects, handles 404
    // (absent → both roots default to the discovery root) and any
    // other non-2xx (hard error).
    //
    // Discovery runs against the *full* auth policy (basic, bearer, or
    // any other scheme the user configured), not the publish-only
    // bearer subset. RFC 8615 does not prevent well-known URIs from
    // being auth-gated, and any auth strategy that works for the rest
    // of the index should work here too.
    let endpoints = runtime.block_on(fetch_well_known(&client, &*auth_policy, &index))?;
    // Only now — after discovery has had access to the full policy —
    // do we consume the Arc to extract the publish-specific
    // bearer-credential map. Upload is bearer-only; basic-auth entries
    // are intentionally dropped at this step.
    let bearer_map = Arc::unwrap_or_clone(auth_policy).try_into_publish_bearer_auth_map()?;
    let api_root = endpoints.api_root.clone();
    let upload_url = build_upload_url(&api_root)?;
    let bearer = match bearer_map.lookup(upload_url.as_str()) {
        GlobMapResult::Found(_, token) => token.clone(),
        GlobMapResult::Ambiguous(candidates) => {
            // Publish must resolve to exactly one bearer token. Unlike the
            // general fetch/auth flow, do not probe multiple credentials here:
            // we do not want to retry uploads or accidentally send unrelated
            // publish credentials to the endpoint. A future refinement could
            // prefer the most specific glob match, which would support
            // separate read and publish credentials under the same host.
            bail!(
                "multiple bearer token credentials configured for publish URL `{upload_url}`; \
                 refine SYSAND_CRED_<X> URL patterns so exactly one bearer token matches ({} candidates found)",
                candidates.len()
            );
        }
        GlobMapResult::NotFound => {
            bail!(
                "no bearer token credentials configured for publish URL `{upload_url}`; \
                 set SYSAND_CRED_<X> and SYSAND_CRED_<X>_BEARER_TOKEN with a matching URL pattern"
            );
        }
    };

    let response = do_publish(prepared, index, api_root, bearer, client, runtime)?;

    let header = sysand_core::style::get_style_config().header;
    if response.is_new_project {
        log::info!(
            "{header}{:>12}{header:#} new project successfully",
            "Published"
        );
    } else {
        log::info!(
            "{header}{:>12}{header:#} new release successfully",
            "Published"
        );
    }

    Ok(())
}

fn resolve_publish_kpar_path(
    path: Option<Utf8PathBuf>,
    ctx: &ProjectContext,
) -> Result<Utf8PathBuf> {
    if let Some(path) = path {
        return Ok(path);
    }

    // Without an explicit path, publish must resolve one concrete project artifact.
    // If no current project is discovered but a workspace is, this is ambiguous
    // (workspace-level context may contain multiple projects), so require `[PATH]`.
    let current_project = match (ctx.current_project.as_ref(), ctx.current_workspace.as_ref()) {
        (Some(current_project), _) => current_project,
        (None, Some(_)) => {
            bail!(
                "`sysand publish` without [PATH] is not supported from a workspace; \
                 run the command from a project directory or pass an explicit .kpar path"
            );
        }
        (None, None) => return Err(CliError::MissingProjectCurrentDir.into()),
    };

    Ok(default_kpar_path(
        current_project,
        ctx.current_workspace.as_ref(),
        &current_project.project_path,
    )?)
}
