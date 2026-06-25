// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::{env, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8PathBuf;
use reqwest::header;
use serde_json::Value;
use sysand_core::{
    auth::{ForceBearerAuth, GlobMapResult, StandardHTTPAuthentication},
    build::default_kpar_path,
    commands::publish::{
        EndpointKind, build_upload_url, do_publish, prepare_publish_payload,
        validate_endpoint_url_shape,
    },
    context::ProjectContext,
    env::discovery::{ResolvedEndpoints, fetch_index_config},
    project::utils::wrapfs,
};
use url::Url;

use crate::{CliError, cli::TrustedPublishingMode};

pub fn command_publish(
    path: Option<Utf8PathBuf>,
    index: Url,
    trusted_publishing: TrustedPublishingMode,
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

    // Resolve `api_root` before credential matching so publish credentials
    // are matched against the actual upload URL. Discovery uses the full auth
    // policy because the discovery document may itself be auth-gated.
    let endpoints = runtime.block_on(fetch_index_config(&client, &*auth_policy, &index))?;
    // Only now — after discovery has had access to the full policy —
    // do we consume the Arc to extract the publish-specific
    // bearer-credential map. Upload is bearer-only; basic-auth entries
    // are intentionally dropped at this step.
    let bearer_map = Arc::unwrap_or_clone(auth_policy).try_into_publish_bearer_auth_map()?;
    let ResolvedEndpoints { api_root, .. } = endpoints;
    let upload_url = build_upload_url(&api_root)?;
    let bearer = select_publish_bearer(
        &bearer_map,
        &upload_url,
        &api_root,
        trusted_publishing,
        &client,
        &runtime,
    )?;

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

/// Choose the bearer token used for upload: explicit matching publish
/// credentials have priority, ambiguous explicit credentials remain an error,
/// and trusted publishing is tried only when no explicit bearer matches.
fn select_publish_bearer(
    bearer_map: &sysand_core::auth::GlobMap<ForceBearerAuth>,
    upload_url: &Url,
    api_root: &Url,
    trusted_publishing: TrustedPublishingMode,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<ForceBearerAuth> {
    match bearer_map.lookup(upload_url.as_str()) {
        GlobMapResult::Found(_, token) => Ok(token.clone()),
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
            match acquire_trusted_publishing_bearer(trusted_publishing, api_root, client, runtime)?
            {
                Some(token) => Ok(token),
                None => {
                    bail!(
                        "no bearer token credentials configured for publish URL `{upload_url}`; \
                         set SYSAND_CRED_<X> and SYSAND_CRED_<X>_BEARER_TOKEN with a matching URL pattern"
                    );
                }
            }
        }
    }
}

/// Trusted publishing providers whose CI environments the publish command can
/// currently recognize and exchange for a Sysand index bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustedPublishingProvider {
    Github,
    Gitlab,
}

/// Resolve a publish bearer token from trusted publishing, returning `None`
/// when the selected mode intentionally leaves publish credential selection to
/// explicit `SYSAND_CRED_*` bearer variables.
fn acquire_trusted_publishing_bearer(
    mode: TrustedPublishingMode,
    api_root: &Url,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<Option<ForceBearerAuth>> {
    let Some(provider) = select_trusted_publishing_provider(mode)? else {
        return Ok(None);
    };

    log::debug!("trusted publishing: using {provider:?}");
    let provider_token = match provider {
        TrustedPublishingProvider::Github => acquire_github_oidc_token(client, runtime)?,
        TrustedPublishingProvider::Gitlab => gitlab_oidc_token_from_env()?,
    };
    let index_token =
        exchange_oidc_token_for_index_token(api_root, &provider_token, client, runtime)
            .with_context(|| format!("trusted publishing with {provider:?} failed"))?;

    Ok(Some(ForceBearerAuth::new(index_token)))
}

/// Convert the CLI trusted-publishing mode into one concrete provider. In
/// `auto` mode incomplete environments are ignored, but two complete provider
/// environments are rejected to avoid guessing which identity to use.
fn select_trusted_publishing_provider(
    mode: TrustedPublishingMode,
) -> Result<Option<TrustedPublishingProvider>> {
    match mode {
        TrustedPublishingMode::Never => Ok(None),
        TrustedPublishingMode::Github => {
            ensure_github_env()?;
            Ok(Some(TrustedPublishingProvider::Github))
        }
        TrustedPublishingMode::Gitlab => {
            ensure_gitlab_env()?;
            Ok(Some(TrustedPublishingProvider::Gitlab))
        }
        TrustedPublishingMode::Auto => {
            let github = github_env_complete();
            let gitlab = gitlab_env_complete();
            match (github, gitlab) {
                (true, true) => bail!(
                    "multiple trusted publishing CI environments detected; specify \
                     --trusted-publishing=github or --trusted-publishing=gitlab"
                ),
                (true, false) => Ok(Some(TrustedPublishingProvider::Github)),
                (false, true) => Ok(Some(TrustedPublishingProvider::Gitlab)),
                (false, false) => Ok(None),
            }
        }
    }
}

/// Whether the GitHub Actions environment exposes the complete OIDC request
/// contract needed to mint a provider token.
fn github_env_complete() -> bool {
    env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_TOKEN").is_some()
        && env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_URL").is_some()
}

/// Whether GitLab CI has injected the configured ID token into the expected
/// job variable.
fn gitlab_env_complete() -> bool {
    env_var_nonempty("GITLAB_OIDC_TOKEN").is_some()
}

/// Validate that forced GitHub trusted publishing has all runner-provided
/// variables needed to request the GitHub OIDC token.
fn ensure_github_env() -> Result<()> {
    let missing: Vec<&str> = [
        "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
        "ACTIONS_ID_TOKEN_REQUEST_URL",
    ]
    .into_iter()
    .filter(|name| env_var_nonempty(name).is_none())
    .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        bail!(
            "trusted publishing provider `github` requires environment variable{} {}",
            if missing.len() == 1 { "" } else { "s" },
            missing.join(", ")
        );
    }
}

/// Validate that forced GitLab trusted publishing has the configured ID token
/// available in the job environment.
fn ensure_gitlab_env() -> Result<()> {
    if gitlab_env_complete() {
        Ok(())
    } else {
        bail!(
            "trusted publishing provider `gitlab` requires environment variable GITLAB_OIDC_TOKEN"
        );
    }
}

/// Read an environment variable while treating an empty string the same as an
/// unset variable.
fn env_var_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Read the GitLab CI OIDC token that GitLab injects when the job declares an
/// `id_tokens` entry for `GITLAB_OIDC_TOKEN`.
fn gitlab_oidc_token_from_env() -> Result<String> {
    env_var_nonempty("GITLAB_OIDC_TOKEN")
        .ok_or_else(|| anyhow!("trusted publishing provider `gitlab` requires GITLAB_OIDC_TOKEN"))
}

/// Ask the GitHub Actions runner OIDC endpoint for a token with the `sysand`
/// audience and return the JSON `value` field from the response.
fn acquire_github_oidc_token(
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<String> {
    ensure_github_env()?;
    let request_token = env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_TOKEN").unwrap();
    let mut request_url = Url::parse(&env_var_nonempty("ACTIONS_ID_TOKEN_REQUEST_URL").unwrap())
        .context("trusted publishing provider `github` has invalid ACTIONS_ID_TOKEN_REQUEST_URL")?;
    request_url
        .query_pairs_mut()
        .append_pair("audience", "sysand");

    let response = runtime.block_on(async {
        client
            .get(request_url.clone())
            .header(header::AUTHORIZATION, format!("bearer {request_token}"))
            .send()
            .await
    })?;

    let status = response.status();
    let body = runtime.block_on(response.bytes())?;
    if !status.is_success() {
        bail!(
            "trusted publishing provider `github` failed to acquire OIDC token: HTTP status {}",
            status.as_u16()
        );
    }

    json_string_field(&body, "value")
        .context("trusted publishing provider `github` returned malformed OIDC response")
}

/// Exchange a provider-issued OIDC token at the resolved index API root and
/// return the short-lived Sysand bearer token from the response.
fn exchange_oidc_token_for_index_token(
    api_root: &Url,
    oidc_token: &str,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> Result<String> {
    let exchange_url = url_with_trailing_slash(api_root.clone())
        .join("v1/oidc/token")
        .unwrap();
    let body = serde_json::json!({ "token": oidc_token }).to_string();

    let response = runtime.block_on(async {
        client
            .post(exchange_url.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
    })?;

    let status = response.status();
    let body = runtime.block_on(response.bytes())?;
    if !status.is_success() {
        bail!(
            "trusted publishing token exchange at `{exchange_url}` failed: HTTP status {}",
            status.as_u16()
        );
    }

    json_string_field(&body, "token").with_context(|| {
        format!("trusted publishing token exchange at `{exchange_url}` returned malformed response")
    })
}

/// Extract a required non-empty string field from a small JSON response body.
fn json_string_field(bytes: &[u8], field: &str) -> Result<String> {
    let value: Value = serde_json::from_slice(bytes)?;
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("missing non-empty `{field}` string"))
}

/// Return a URL whose path ends in `/` so `Url::join` appends endpoint paths
/// below the API root instead of replacing the last path segment.
fn url_with_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url
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
