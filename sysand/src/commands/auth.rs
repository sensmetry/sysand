// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI wrappers for the `sysand auth` commands (design/credential-storage.md
//! sections 4, 9): default-index resolution, the OS keyring store handoff
//! to core, and user-facing rendering. Secrets never appear in any output.

use std::io::{IsTerminal, Read};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use sysand_core::{
    commands::auth::{
        AuthCommandError, AuthLoginNotice, AuthLoginOutcome, AuthStatus, EnvCredentialEntry,
        ProbeSurface, StoredCredentialsStatus, do_auth_login, do_auth_logout, do_auth_status,
        validated_index_key,
    },
    config::Config,
    credential_store::CredentialStoreError,
};

use crate::{DEFAULT_INDEX_URL, credential_store::open_cli_credential_store};

const KEYRING_LOCKED_HINT: &str = "unlock your OS keyring and retry, or provide credentials via \
     `SYSAND_CRED_*` environment variables";

/// Resolve the single index a bare `sysand auth login` / `auth logout`
/// targets (design/credential-storage.md section 4): the
/// `--default-index` / `SYSAND_DEFAULT_INDEX` override when given, else a
/// `default = true` index from configuration, else the built-in
/// [`DEFAULT_INDEX_URL`]. If the consulted stage yields more than one
/// distinct URL, the target is ambiguous and an explicit URL is required.
pub fn resolve_default_index(default_index_override: &[String], config: &Config) -> Result<String> {
    let mut candidates: Vec<&str> = if default_index_override.is_empty() {
        config
            .indexes
            .iter()
            .filter(|index| index.default.unwrap_or(false))
            .map(|index| index.url.as_str())
            .collect()
    } else {
        default_index_override.iter().map(String::as_str).collect()
    };
    // Exact duplicates of one URL are still a single target.
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|url| seen.insert(*url));

    match candidates.as_slice() {
        [] => Ok(DEFAULT_INDEX_URL.to_string()),
        [single] => Ok((*single).to_string()),
        many => bail!(
            "more than one default index is configured ({}); pass an explicit index URL",
            many.join(", ")
        ),
    }
}

/// Human name of a probe surface for warnings.
fn surface_name(surface: ProbeSurface) -> &'static str {
    match surface {
        ProbeSurface::Read => "index read surface (`index.json`)",
        ProbeSurface::Api => "index API (`v1/whoami`)",
    }
}

pub fn command_auth_login(
    index_url: Option<String>,
    token_stdin: bool,
    validation: Option<bool>,
    default_index: &[String],
    config: &Config,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(default_index, config)?,
    };
    // Validate and normalize before any secret entry, so a `file://` or
    // unanchorable-template target fails without prompting.
    let key = validated_index_key(&target)?;
    // Echo the resolved index before reading the secret, on both the
    // prompt and the stdin path, so a project-configured default cannot
    // be targeted silently (design/credential-storage.md section 4).
    // Printed, not logged: `--quiet` must not hide it.
    println!("Logging in to index `{key}`");

    let secret = read_token(&key, token_stdin)?;

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    let index_key = key.clone();
    let outcome = do_auth_login(
        &mut store,
        &key,
        secret,
        validation,
        client,
        runtime,
        |notice| {
            match notice {
                // Printed before the store write happens.
                AuthLoginNotice::ReplacingExisting { key } => {
                    println!("replacing existing credential for `{key}`");
                }
                AuthLoginNotice::DiscoveryUnreachable { error } => log::warn!(
                    "could not read the index configuration ({error}); \
                     scoping the credential to the URL-derived pattern"
                ),
                AuthLoginNotice::TemplateIndexRootSkipped { template } => log::warn!(
                    "discovery advertises a templated index_root (`{template}`) that \
                     cannot be covered safely; no pattern was derived for it"
                ),
                AuthLoginNotice::ProbeRedirected { surface, target } => log::warn!(
                    "the {} probe was redirected to `{target}`; probes do not follow \
                     redirects, so the credential was not validated against that surface",
                    surface_name(surface)
                ),
                AuthLoginNotice::ProbeUnreachable { surface, error } => log::warn!(
                    "could not probe the {} ({error}); the credential was not \
                     validated against it",
                    surface_name(surface)
                ),
                AuthLoginNotice::ProbeRateLimited { surface } => log::warn!(
                    "the {} probe was rate limited (HTTP 429); the credential was \
                     not validated against it",
                    surface_name(surface)
                ),
                AuthLoginNotice::SurfaceRejected {
                    surface,
                    basic_challenge,
                } => {
                    log::warn!(
                        "the {} rejected the credential; it was stored anyway because \
                         another surface accepted it",
                        surface_name(surface)
                    );
                    if basic_challenge {
                        let stem = cred_env_var_stem(&index_key);
                        log::warn!(
                            "this index uses username/password (HTTP basic) authentication; \
                             configure `SYSAND_CRED_{stem}_BASIC_USER` / \
                             `SYSAND_CRED_{stem}_BASIC_PASS` environment variables instead"
                        );
                    }
                }
            }
        },
    );
    match outcome {
        Ok(AuthLoginOutcome::Stored {
            key,
            globs,
            validated,
        }) => {
            let header = sysand_core::style::get_style_config().header;
            // The claim is always scoped to the surfaces that accepted
            // the credential; never a bare "validated"
            // (design/credential-storage.md section 5).
            let claim = if validated.is_empty() {
                "stored, not validated".to_string()
            } else {
                let surfaces: Vec<String> = validated
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect();
                format!("validated ({})", surfaces.join(", "))
            };
            log::info!(
                "{header}{:>12}{header:#} credential for `{key}` ({claim})",
                "Stored"
            );
            println!("The credential covers: {}", globs.join(", "));
            Ok(())
        }
        Ok(AuthLoginOutcome::BackendUnavailable { key, globs, reason }) => {
            // No-keyring host (design/credential-storage.md section 9):
            // refuse to persist, print the exact `SYSAND_CRED_*` lines to
            // set instead. The secret is never echoed: stdout on such
            // hosts typically lands in captured CI job logs.
            println!(
                "no OS keyring backend is available ({reason}), so the credential \
                 was not stored."
            );
            println!(
                "To authenticate on this host, set these environment variables \
                 instead, replacing `<token>` with the token you entered:"
            );
            let stem = cred_env_var_stem(&key);
            for (position, glob) in globs.iter().enumerate() {
                let name = if position == 0 {
                    stem.clone()
                } else {
                    format!("{}_{}", stem, position + 1)
                };
                println!("  SYSAND_CRED_{name}={glob}");
                println!("  SYSAND_CRED_{name}_BEARER_TOKEN=<token>");
            }
            bail!("credential not stored: no OS keyring backend on this host");
        }
        Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
            Err(err).context(KEYRING_LOCKED_HINT)
        }
        Err(err) => Err(err.into()),
    }
}

/// Read the login secret: from stdin when `--token-stdin` was given
/// (trimming exactly one trailing newline), else from a hidden terminal
/// prompt. Fails fast when stdin is not a terminal and `--token-stdin`
/// was not given, instead of hanging or reading a pipe as a secret.
fn read_token(key: &str, token_stdin: bool) -> Result<String> {
    let token = if token_stdin {
        let mut raw = String::new();
        std::io::stdin()
            .read_to_string(&mut raw)
            .context("could not read the token from stdin")?;
        let trimmed = match raw.strip_suffix('\n') {
            Some(rest) => rest.strip_suffix('\r').unwrap_or(rest),
            None => raw.as_str(),
        };
        trimmed.to_string()
    } else if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("Enter token for `{key}`:"))
            .context("could not read the token from the terminal")?
    } else {
        bail!("no terminal for prompt; pass the token with `--token-stdin`");
    };
    if token.is_empty() {
        bail!("empty token; nothing was stored");
    }
    Ok(token)
}

/// Derive the `SYSAND_CRED_<NAME>` stem suggested on a no-keyring host
/// from the index host. Hostnames cannot produce the reserved
/// `_BASIC_USER` / `_BASIC_PASS` / `_BEARER_TOKEN` suffixes.
fn cred_env_var_stem(key: &str) -> String {
    let host = url::Url::parse(key)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "INDEX".to_string());
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub fn command_auth_logout(
    index_url: Option<String>,
    default_index: &[String],
    config: &Config,
) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(default_index, config)?,
    };
    // Echo the resolved index (normalized to the stored-key form when
    // possible) so a configured default cannot be targeted silently.
    // Printed, not logged: `--quiet` must not hide it.
    let echo = validated_index_key(&target).unwrap_or_else(|_| target.clone());
    println!("Logging out from index `{echo}`");

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    match do_auth_logout(&mut store, &target) {
        Ok(key) => {
            let header = sysand_core::style::get_style_config().header;
            log::info!(
                "{header}{:>12}{header:#} stored credential for `{key}`",
                "Removed"
            );
            Ok(())
        }
        Err(AuthCommandError::Store(CredentialStoreError::BackendAbsent { source })) => bail!(
            "no OS keyring backend is available ({source}), so this host has no \
             stored logins; credentials here come from `SYSAND_CRED_*` \
             environment variables"
        ),
        Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
            Err(err).context(KEYRING_LOCKED_HINT)
        }
        Err(err) => Err(err.into()),
    }
}

pub fn command_auth_status() -> Result<()> {
    let env = collect_env_credential_entries();
    let status = match open_cli_credential_store() {
        Ok(store) => match do_auth_status(&store, env) {
            Ok(status) => status,
            Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
                return Err(err).context(KEYRING_LOCKED_HINT);
            }
            Err(err) => return Err(err.into()),
        },
        // Could not even open the store (no per-user lock path): degrade
        // to the env-only view like an absent backend.
        Err(err) => AuthStatus {
            stored: StoredCredentialsStatus::BackendUnavailable {
                reason: err.to_string(),
            },
            env: collect_env_credential_entries(),
        },
    };
    render_auth_status(&status);
    Ok(())
}

/// Collect `SYSAND_CRED_*` URL-pattern variables as status entries.
///
/// Deliberately tolerant, unlike the eager auth-policy build: `auth
/// status` is the command for diagnosing credential configuration, so an
/// incomplete group (a pattern without a scheme variable) is still listed
/// rather than rejected. Only pattern variables are shown; the `_BASIC_USER`
/// / `_BASIC_PASS` / `_BEARER_TOKEN` companions hold secrets.
fn collect_env_credential_entries() -> Vec<EnvCredentialEntry> {
    let mut entries: Vec<EnvCredentialEntry> = std::env::vars()
        .filter(|(key, _)| {
            key.strip_prefix("SYSAND_CRED_").is_some_and(|rest| {
                !rest.ends_with("_BASIC_USER")
                    && !rest.ends_with("_BASIC_PASS")
                    && !rest.ends_with("_BEARER_TOKEN")
            })
        })
        .map(|(key, value)| EnvCredentialEntry {
            label: key,
            pattern: value,
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

/// Render the unified status view: stored entries tagged `stored` (key in
/// the exact form `sysand auth logout <key>` accepts), env entries tagged
/// `env`. Never any secret.
fn render_auth_status(status: &AuthStatus) {
    match &status.stored {
        StoredCredentialsStatus::BackendUnavailable { reason } => {
            println!(
                "note: no usable OS keyring backend ({reason}); showing \
                 `SYSAND_CRED_*` environment credentials only"
            );
        }
        StoredCredentialsStatus::Available(stored) if stored.is_empty() => {
            println!("No stored index logins.");
        }
        StoredCredentialsStatus::Available(stored) => {
            for entry in stored {
                println!("stored  {}", entry.key);
                println!("        patterns: {}", entry.globs.join(", "));
                if let Some(subject) = &entry.subject {
                    println!("        subject: {} {}", subject.kind, subject.name);
                }
                if let Some(prefix) = &entry.token_prefix {
                    println!("        token prefix: {prefix}");
                }
                if let Some(expires_at) = &entry.expires_at {
                    let qualifier = if entry.expired {
                        " (expired)".to_string()
                    } else {
                        match entry.expires_in_days {
                            Some(1) => " (expires in 1 day)".to_string(),
                            Some(days) => format!(" (expires in {days} days)"),
                            None => String::new(),
                        }
                    };
                    println!("        expires: {expires_at}{qualifier}");
                }
                if !entry.shadowed_by.is_empty() {
                    println!("        shadowed by: {}", entry.shadowed_by.join(", "));
                }
            }
        }
    }
    if status.env.is_empty() {
        println!("No `SYSAND_CRED_*` environment credentials.");
    } else {
        for entry in &status.env {
            println!("env     {}  {}", entry.label, entry.pattern);
        }
    }
}
