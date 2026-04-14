// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{
    auth::{GlobMapResult, StandardHTTPAuthentication},
    build::default_kpar_path,
    commands::publish::{build_upload_url, do_publish},
    context::ProjectContext,
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
    // Consume the Arc (or clone if shared) to extract owned credentials.
    let bearer_map = Arc::unwrap_or_clone(auth_policy).try_into_publish_bearer_auth_map()?;

    // Match credentials against the concrete upload endpoint, not the index root,
    // so users can scope patterns to `/api/v1/upload` when needed.
    let upload_url = build_upload_url(&index)?;
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

    let response = do_publish(kpar_path, index, bearer, client, runtime)?;

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
