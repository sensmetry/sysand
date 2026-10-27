// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use std::sync::Arc;

use anyhow::{Result, bail};
use camino::Utf8PathBuf;
use sysand_core::{
    build::{default_kpar_path, workspace_publish_order},
    commands::publish::{
        PublishBearerProvenance, PublishError, TrustedPublishingEnvironment, do_publish,
        prepare_publish_payload, resolve_publish_bearer, validate_api_root_url_shape,
    },
    context::ProjectContext,
    env::discovery::{ResolvedEndpoints, fetch_index_config},
    index_location::IndexSource,
    project::utils::wrapfs,
    workspace::model_toml::{MemberChecks, Workspace},
};

use crate::{CliAuthPolicy, CliError, cli::TrustedPublishingModeCli};

pub fn command_publish(
    path: Option<Utf8PathBuf>,
    index: IndexSource,
    trusted_publishing: TrustedPublishingModeCli,
    ctx: &ProjectContext,
    auth_policy: Arc<CliAuthPolicy>,
    client: reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    // A directory has no upload endpoint; `sysand index add` is how an
    // archive enters a filesystem index. Refused before any archive is
    // touched, so the message is the first and only one.
    let index = match index {
        IndexSource::Remote(index) => index,
        IndexSource::Filesystem(path) => bail!(
            "publishing to a filesystem index is not supported;\n\
             add archives to `{path}` with `sysand index add`"
        ),
    };
    let archives = resolve_publish_archives(path, ctx)?;
    // Validate and prepare every payload before any network work, so
    // kpar-content errors surface before discovery or credential
    // matching -- and, for a workspace, before anything at all is
    // uploaded. Discovering a malformed archive after uploading three
    // others is not a state to leave a user in.
    let mut prepared = Vec::with_capacity(archives.len());
    for kpar_path in &archives {
        if !wrapfs::is_file(kpar_path)? {
            bail!("KPAR file not found at `{kpar_path}`, run `sysand build` first");
        }
        prepared.push(prepare_publish_payload(kpar_path)?);
    }

    // Resolve `api_root` before credential matching so publish credentials
    // are matched against the actual upload URL. Discovery uses the full
    // auth policy because the discovery document may itself be auth-gated.
    let endpoints = runtime.block_on(fetch_index_config(&client, &*auth_policy, &index))?;
    let ResolvedEndpoints {
        api_root,
        index_root: _,
    } = endpoints;
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

    let header = sysand_core::style::get_style_config().header;
    let mut published: Vec<String> = Vec::new();
    for (payload, kpar_path) in prepared.into_iter().zip(&archives) {
        let name = payload.identity();
        let response = do_publish(
            payload,
            index.clone(),
            api_root.clone(),
            bearer.clone(),
            client.clone(),
            runtime.clone(),
        )
        .map_err(|err| {
            // A failure stops the sequence and reports what was
            // published: the uploads that succeeded are not undone, so
            // the user has to be told which they were.
            let err = publish_error_with_hint(err, &index_display);
            if published.is_empty() {
                err
            } else {
                err.context(format!(
                    "already published: {}; `{kpar_path}` and any after it were not",
                    published.join(", ")
                ))
            }
        })?;

        // The name, because one archive's "Published new project
        // successfully" is fine and five of them are not.
        let what = if response.is_new_project {
            "new project"
        } else {
            "new release"
        };
        log::info!("{header}{:>12}{header:#} {name} as a {what}", "Published");
        published.push(name);
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
        PublishError::NoPublishBearer { upload_url: _ } => {
            Some(format!("or run `sysand auth login {index}` to store one"))
        }
        PublishError::StoredCredentialExpired { key, expires_at: _ } => Some(format!(
            "re-run `sysand auth login {key}` to store a fresh token"
        )),
        PublishError::PublishAuthFailed {
            status,
            provenance,
            detail: _,
        } => {
            // Re-login refreshes a stored credential but not an env one
            // (the core message explains the shadowing). A 403 is
            // authorization: point at the credential's subject.
            let mut lines = Vec::new();
            if let PublishBearerProvenance::Stored { key, expires_at: _ } = provenance {
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

/// The archives to publish, in the order they must go.
///
/// One for an explicit `[PATH]` or for a project directory -- a
/// workspace member included, which resolves to the member's own
/// archive under the workspace `output/`. From a workspace root with no
/// project, every member in dependency order.
fn resolve_publish_archives(
    path: Option<Utf8PathBuf>,
    ctx: &ProjectContext,
) -> Result<Vec<Utf8PathBuf>> {
    if let Some(path) = path {
        return Ok(vec![path]);
    }

    match (
        ctx.current_project.as_ref(),
        ctx.model_workspace_root.as_ref(),
    ) {
        // A member publishes the archive `sysand build` wrote for it,
        // which is in the workspace's single `output/`.
        (Some(current_project), workspace_root) => Ok(vec![default_kpar_path(
            current_project,
            workspace_root.map(camino::Utf8PathBuf::as_path),
            current_project.root_path(),
        )?]),
        // From a workspace root, every member, in dependency order.
        (None, Some(root)) => {
            log::warn!(
                "Workspaces are an experimental feature\n\
                and their behavior may change even with minor\n\
                releases. For the status of this feature, see\n\
                https://github.com/sensmetry/sysand/issues/101."
            );
            let workspace = Workspace::load_with(root, MemberChecks::All)?;
            Ok(workspace_publish_order(&workspace)
                .map_err(|failure| anyhow::anyhow!("{failure}"))?
                .into_iter()
                .map(|(_, archive)| archive)
                .collect())
        }
        (None, None) => Err(CliError::MissingProjectCurrentDir.into()),
    }
}
