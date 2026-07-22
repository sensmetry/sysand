// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{
    build::default_kpar_path,
    commands::publish::{
        PublishBearerProvenance, PublishError, TrustedPublishingEnvironment, do_publish,
        prepare_publish_payload, resolve_publish_bearer, validate_api_root_url_shape,
    },
    context::ProjectContext,
    env::discovery::{ResolvedEndpoints, fetch_index_config},
    index_location::IndexLocation,
    project::utils::wrapfs,
};

use crate::{CliAuthPolicy, CliError, cli::TrustedPublishingMode};

pub fn command_publish(
    path: Option<Utf8PathBuf>,
    index: IndexLocation,
    trusted_publishing: TrustedPublishingMode,
    ctx: &ProjectContext,
    auth_policy: Arc<CliAuthPolicy>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let kpar_path = resolve_publish_kpar_path(path, ctx)?;
    if !wrapfs::is_file(&kpar_path)? {
        bail!("KPAR file not found at `{kpar_path}`, run `sysand build` first");
    }
    // `index` is an `IndexLocation`, so its shape invariants (absolute
    // HTTP(S), no userinfo) were already enforced when it was parsed. The
    // `v1/upload` guard still applies to the resolved `api_root` in
    // `build_upload_url`.
    //
    // Validate and prepare the kpar payload before any network work,
    // so that kpar-content errors (bad semver, invalid publisher/name,
    // oversized archive) surface before discovery or credential
    // matching does.
    let prepared = prepare_publish_payload(&kpar_path)?;

    // Resolve `api_root` before credential matching so publish credentials
    // are matched against the actual upload URL. Discovery uses the full auth
    // policy because the discovery document may itself be auth-gated.
    let endpoints = runtime.block_on(fetch_index_config(&client, &*auth_policy, &index))?;
    let ResolvedEndpoints { api_root, .. } = endpoints;
    // Publishing needs an API, which exists only when the discovery
    // document advertises `api_root`. An index that does not advertise one
    // serves files only, so there is nothing to upload to. Check before
    // credential handling so this clearer error is not masked by
    // credential problems.
    let Some(api_root) = api_root else {
        bail!(
            "index `{index}` does not advertise a publish endpoint,\n\
             so publishing to it is not supported; ask the index administrator\n\
             whether publishing is available (an index accepts uploads only\n\
             if its sysand-index-config.json sets `api_root`)"
        );
    };
    // Validate the resolved `api_root` shape once here; both credential
    // resolution and the upload build the upload URL from it afterwards.
    validate_api_root_url_shape(&api_root)?;
    // Only now, after discovery has had access to the full policy, do we
    // extract the publish-specific bearer-credential map (by reference,
    // cloning the tokens). Upload is bearer-only; basic-auth entries are
    // intentionally dropped at this step. Stored logins are handed over as
    // a lazy provider so the credential store is read (once, cached on the
    // policy) only when no env bearer matches the upload URL.
    let env_bearers = auth_policy.env_policy().publish_bearer_auth_map()?;
    let trusted_publishing_env = TrustedPublishingEnvironment::from_env();
    let bearer = resolve_publish_bearer(
        &env_bearers,
        || auth_policy.stored_bearer_map_blocking().clone(),
        &api_root,
        trusted_publishing.into(),
        &trusted_publishing_env,
        &client,
        &runtime,
    )
    .map_err(publish_error_with_hint)?;

    let response = do_publish(prepared, index, api_root, bearer, client, runtime)
        .map_err(publish_error_with_hint)?;

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

/// Add the CLI-specific `sysand auth` remediation to a publish error whose
/// core message deliberately names no CLI command: the library states the
/// condition (and the `SYSAND_CRED_*` fallback), and the frontend owns the
/// command vocabulary.
fn publish_error_with_hint(err: PublishError) -> anyhow::Error {
    let hint = match &err {
        PublishError::NoPublishBearer { .. } => {
            Some("or run `sysand auth login <index-url>` to store one".to_string())
        }
        PublishError::StoredCredentialExpired { key, .. } => Some(format!(
            "re-run `sysand auth login {key}` to store a fresh token"
        )),
        PublishError::PublishAuthFailed {
            status, provenance, ..
        } => {
            // A stale stored credential is refreshed by re-login; an env
            // credential is not (it shadows stored logins, which the core
            // message already explains). A 403 is authorization, so point
            // at the subject the credential authenticates as.
            let mut lines = Vec::new();
            if let PublishBearerProvenance::Stored { key, .. } = provenance {
                lines.push(format!(
                    "re-run `sysand auth login {key}` to store a fresh token"
                ));
            }
            if *status == 403 {
                lines.push(
                    "run `sysand auth status` to see which subject the credential \
                     authenticates as"
                        .to_string(),
                );
            }
            (!lines.is_empty()).then(|| lines.join("\n"))
        }
        _ => None,
    };
    match hint {
        Some(hint) => anyhow::anyhow!("{err}\n{hint}"),
        None => err.into(),
    }
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
        current_project.root_path(),
    )?)
}
