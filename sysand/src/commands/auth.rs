// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI wrappers for the `sysand auth` commands (design/credential-storage.md
//! sections 4, 9): default-index resolution, the OS keyring store handoff
//! to core, and user-facing rendering. Secrets never appear in any output.

use anyhow::{Context, Result, bail};
use sysand_core::{
    commands::auth::{
        AuthCommandError, AuthStatus, EnvCredentialEntry, StoredCredentialsStatus, do_auth_logout,
        do_auth_status,
    },
    config::Config,
    credential_store::{
        CredentialStoreError, keyring_store::KeyringCredentialStore, normalize_index_key,
    },
};

use crate::DEFAULT_INDEX_URL;

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
    let echo = normalize_index_key(&target).unwrap_or_else(|_| target.clone());
    println!("Logging out from index `{echo}`");

    let mut store =
        KeyringCredentialStore::open_default().context("could not open the credential store")?;
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
    let status = match KeyringCredentialStore::open_default() {
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
                if let Some(expires_at) = &entry.expires_at {
                    let expired = if entry.expired { " (expired)" } else { "" };
                    println!("        expires: {expires_at}{expired}");
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
