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
    // Validate and prepare the kpar payload before any network work, so
    // kpar-content errors surface before discovery or credential matching.
    let prepared = prepare_publish_payload(&kpar_path)?;

    // Resolve `api_root` before credential matching so publish credentials
    // are matched against the actual upload URL. Discovery uses the full
    // auth policy because the discovery document may itself be auth-gated.
    let endpoints = runtime.block_on(fetch_index_config(&client, &*auth_policy, &index))?;
    let ResolvedEndpoints { api_root, .. } = endpoints;
    // No advertised `api_root` means a files-only index with nothing to
    // upload to; check before credential handling so this clearer error
    // is not masked by credential problems.
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
    // Upload is bearer-only, so basic-auth entries are dropped here.
    // Stored logins are handed over as a lazy provider: the credential
    // store is read only when no env bearer matches the upload URL.
    let env_bearers = auth_policy.env_policy().publish_bearer_auth_map()?;
    let trusted_publishing_env = TrustedPublishingEnvironment::from_env();
    // Captured before `index` moves into `do_publish`, so both hint sites
    // can name the index the user actually targeted.
    let index_display = index.to_string();
    let bearer = resolve_publish_bearer(
        &env_bearers,
        || auth_policy.read_stored_bearer_map_direct(),
        &api_root,
        trusted_publishing.into(),
        &trusted_publishing_env,
        &client,
        &runtime,
    )
    .map_err(|err| publish_error_with_hint(err, &index_display))?;

    let response = do_publish(prepared, index, api_root, bearer, client, runtime)
        .map_err(|err| publish_error_with_hint(err, &index_display))?;

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
/// command vocabulary. `index` is the index the user targeted, so the
/// login hint is copy-pasteable.
fn publish_error_with_hint(err: PublishError, index: &str) -> anyhow::Error {
    let hint = match &err {
        PublishError::NoPublishBearer { .. } => {
            Some(format!("or run `sysand auth login {index}` to store one"))
        }
        PublishError::StoredCredentialExpired { key, .. } => Some(format!(
            "re-run `sysand auth login {key}` to store a fresh token"
        )),
        PublishError::PublishAuthFailed {
            status, provenance, ..
        } => {
            // Re-login refreshes a stored credential but not an env one
            // (the core message explains the shadowing). A 403 is
            // authorization: point at the credential's subject.
            let mut lines = Vec::new();
            if let PublishBearerProvenance::Stored { key, .. } = provenance {
                lines.push(format!(
                    "re-run `sysand auth login {key}` to store a fresh token"
                ));
            }
            if *status == 403 {
                lines.push(
                    "run `sysand auth status` to see which subject the credential\n\
                     authenticates as"
                        .to_owned(),
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
