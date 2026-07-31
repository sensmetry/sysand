// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI wrappers for the `sysand auth` commands: default-index
//! resolution, the OS keyring store handoff to core, and user-facing
//! rendering. Secrets never appear in any output.

use std::io::{IsTerminal, Read};
use std::sync::Arc;

use anstream::println;
use anyhow::{Context, Result, bail};
use sysand_core::{
    auth::{GlobMapBuilder, StandardHTTPAuthentication, StandardHTTPAuthenticationBuilder},
    commands::auth::{
        AuthCommandError, AuthLoginNotice, AuthLoginOutcome, AuthStatus, EnvCredentialEntry,
        IndexKey, ProbeSurface, StoredCredentialStatus, StoredCredentialsStatus,
        WhoamiCredentialSource, WhoamiVerdict, assemble_auth_status, do_auth_login, do_auth_logout,
        do_auth_status, do_auth_whoami,
    },
    config::Config,
    credential_store::CredentialStoreError,
};

use chrono::{DateTime, Utc};

use crate::{CliAuthPolicy, DEFAULT_INDEX_URL, credential_store::open_cli_credential_store};

const KEYRING_LOCKED_HINT: &str = "unlock your OS keyring and retry, or provide credentials via\n\
     `SYSAND_CRED_*` environment variables";

/// Resolve the single index a bare `sysand auth login` / `auth logout`
/// targets: the `SYSAND_DEFAULT_INDEX` environment override
/// (comma-delimited) when
/// set, else a `default = true` index from configuration, else the
/// built-in [`DEFAULT_INDEX_URL`]. If the consulted stage yields more
/// than one distinct URL, the target is ambiguous and an explicit URL is
/// required.
pub fn resolve_default_index(config: &Config) -> Result<String> {
    let env_override = std::env::var(crate::env_vars::SYSAND_DEFAULT_INDEX).ok();
    let env_candidates: Vec<&str> = env_override
        .as_deref()
        .map(|raw| raw.split(',').filter(|url| !url.is_empty()).collect())
        .unwrap_or_default();
    let mut candidates: Vec<&str> = if env_candidates.is_empty() {
        config
            .indexes
            .iter()
            .filter(|index| index.default.unwrap_or(false))
            .map(|index| index.url.as_str())
            .collect()
    } else {
        env_candidates
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

/// Render a login progress notice. `ReplacingExisting` is informational
/// (log output, suppressed under `--quiet`, like the `Stored`/`Covers`
/// result lines); the rest are warnings. `index_key` names the login for
/// the basic-auth hint.
fn render_login_notice(notice: AuthLoginNotice, index_key: &str) {
    match notice {
        AuthLoginNotice::ReplacingExisting { key } => {
            let header = sysand_core::style::get_style_config().header;
            log::info!(
                "{header}{:>12}{header:#} existing credential for `{key}`",
                "Replacing"
            );
        }
        AuthLoginNotice::DiscoveryUnreachable { error } => log::warn!(
            "could not read the index configuration ({error});\n\
                 scoping the credential to the URL-derived pattern"
        ),
        AuthLoginNotice::TemplateIndexRootSkipped { template } => log::warn!(
            "discovery advertises a templated index_root (`{template}`) that\n\
                 cannot be covered safely; no pattern was derived for it"
        ),
        AuthLoginNotice::ProbeRedirected { surface, target } => log::warn!(
            "the {} probe was redirected to `{target}`; probes do not follow\n\
                 redirects, so the credential was not validated against that surface",
            surface_name(surface)
        ),
        AuthLoginNotice::ProbeUnreachable { surface, error } => log::warn!(
            "could not probe the {} ({error}); the credential was not\n\
                 validated against it",
            surface_name(surface)
        ),
        AuthLoginNotice::ProbeRateLimited { surface } => log::warn!(
            "the {} probe was rate limited (HTTP 429); the credential was\n\
                 not validated against it",
            surface_name(surface)
        ),
        AuthLoginNotice::SurfaceRejected {
            surface,
            status,
            basic_challenge,
        } => {
            // Same read-404 hedge as the refusal message.
            let hedge = if surface == ProbeSurface::Read && status == 404 {
                ", which can also mean no index exists at this URL"
            } else {
                ""
            };
            log::warn!(
                "the {} rejected the credential (HTTP {status}{hedge});\n\
                     it was stored anyway because another surface accepted it",
                surface_name(surface)
            );
            if basic_challenge {
                let stem = cred_env_var_stem(index_key);
                // Keep this basic-auth routing hint consistent with the
                // refusal-path variant in core/src/commands/auth.rs
                // (validation_rejected_message).
                log::warn!(
                    "this index uses username/password (HTTP basic) authentication;\n\
                         configure `SYSAND_CRED_{stem}_BASIC_USER` /\n\
                         `SYSAND_CRED_{stem}_BASIC_PASS` environment variables instead"
                );
            }
        }
    }
}

pub fn command_auth_login(
    index_url: Option<String>,
    token_stdin: bool,
    config: &Config,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: Arc<tokio::runtime::Runtime>,
) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(config)?,
    };
    // Validate and normalize before any secret entry, so a `file://` or
    // unanchorable-template target fails without prompting. This is the
    // one validation; core takes the resulting `IndexKey` as proof.
    let key = IndexKey::validate(&target)?;
    // Echo the resolved index before reading the secret, on both the
    // prompt and the stdin path, so a project-configured default cannot
    // be targeted silently.
    // Printed, not logged: `--quiet` must not hide it.
    let header = sysand_core::style::get_style_config().header;
    println!("{header}{:>12}{header:#} to index `{key}`", "Logging in");

    // An `http` index sends the token in cleartext, and a MITM at login
    // could persist a hostile `api_root`. Warn before the secret is
    // entered so the user can abort.
    if key.as_str().starts_with("http://") {
        log::warn!(
            "`{key}` uses an unencrypted (http) connection; the token will be\n\
             sent in cleartext. Prefer an https:// index."
        );
    }

    let secret = read_token(key.as_str(), token_stdin)?;

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    let index_key = key.to_string();
    let outcome = do_auth_login(&mut store, &key, secret, client, &runtime, |notice| {
        render_login_notice(notice, &index_key);
    });
    match outcome {
        Ok(AuthLoginOutcome::Stored {
            key,
            globs,
            validated,
        }) => {
            // The claim is scoped to the surfaces that accepted; never a
            // bare "validated".
            let claim = if validated.is_empty() {
                "stored, not validated".to_string()
            } else {
                let surfaces: Vec<String> = validated
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect();
                format!("validated ({})", surfaces.join(", "))
            };
            // Both lines are the login result: one channel (the log), so
            // `--quiet` suppresses the confirmation as a unit.
            log::info!(
                "{header}{:>12}{header:#} credential for `{key}` ({claim})",
                "Stored"
            );
            log::info!("{header}{:>12}{header:#} {}", "Covers", globs.join(", "));
            Ok(())
        }
        Ok(AuthLoginOutcome::BackendUnavailable { key, globs, reason }) => {
            // No-keyring host: print the exact
            // `SYSAND_CRED_*` lines to set instead. The secret is never
            // echoed: stdout here typically lands in CI job logs.
            println!(
                "No OS keyring backend is available ({reason}), so the credential\n\
                 was not stored."
            );
            println!(
                "To authenticate on this host, set these environment variables\n\
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
        Err(AuthCommandError::Store(CredentialStoreError::BlobTooLarge)) => {
            // The core store reports only the condition (it must not name
            // CLI commands); the CLI owns the `sysand auth` remediation.
            let stored = store.list().unwrap_or_default();
            let entries: Vec<(&str, Option<DateTime<Utc>>)> = stored
                .iter()
                .map(|record| (record.key.as_str(), record.expires_at))
                .collect();
            bail!("{}", blob_full_message(&entries, Utc::now()))
        }
        Err(err) => Err(err.into()),
    }
}

/// Remediation message for the platform credential-store cap on
/// `auth login` (Windows ~2.5 KB). Naming the `sysand auth` commands
/// belongs in the CLI, not core. Lists already-expired logins as drop
/// candidates; with no stored credentials there is nothing to remove.
fn blob_full_message(stored: &[(&str, Option<DateTime<Utc>>)], now: DateTime<Utc>) -> String {
    let expired: Vec<&str> = stored
        .iter()
        .filter(|(_, at)| at.is_some_and(|at| at < now))
        .map(|(key, _)| *key)
        .collect();
    let mut message = if stored.is_empty() {
        "the token is too large for this platform's credential store\n\
         (Windows ~2.5 KB limit); use a smaller token"
            .to_string()
    } else {
        "the credential store is full (Windows ~2.5 KB limit); remove a login with\n\
         `sysand auth logout <index>` (run `sysand auth status` to list them) or use\n\
         a smaller token"
            .to_string()
    };
    if !expired.is_empty() {
        message.push_str("\nthese stored credentials have expired and are safe to remove:");
        for key in expired {
            message.push_str(&format!("\n  sysand auth logout {key}"));
        }
    }
    message
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
/// from the index host plus any non-default port: two indexes on
/// different ports of one host must not suggest the same variable names.
/// Hostnames and ports cannot produce the reserved `_BASIC_USER` /
/// `_BASIC_PASS` / `_BEARER_TOKEN` suffixes (a port is all digits).
fn cred_env_var_stem(key: &str) -> String {
    let url = url::Url::parse(key).ok();
    let host = url
        .as_ref()
        .and_then(|url| url.host_str())
        .unwrap_or("INDEX");
    let mut stem: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    // `url::Url` strips a scheme-default port during normalization, so
    // `port()` is `Some` exactly for non-default ports.
    if let Some(port) = url.as_ref().and_then(url::Url::port) {
        stem.push('_');
        stem.push_str(&port.to_string());
    }
    stem
}

pub fn command_auth_logout(index_url: Option<String>, config: &Config) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(config)?,
    };
    // Echo the resolved index (normalized to the stored-key form when
    // possible) so a configured default cannot be targeted silently.
    // Printed, not logged: `--quiet` must not hide it.
    let validated = IndexKey::validate(&target);
    let echo = validated.as_ref().map_or(target.as_str(), IndexKey::as_str);
    let header = sysand_core::style::get_style_config().header;
    println!(
        "{header}{:>12}{header:#} from index `{echo}`",
        "Logging out"
    );

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    // The one validation for this command; an invalid target surfaces
    // here, after the echo, as it did when core validated it.
    let key = validated?;
    match do_auth_logout(&mut store, &key) {
        Ok(key) => {
            log::info!(
                "{header}{:>12}{header:#} stored credential for `{key}`",
                "Removed"
            );
            Ok(())
        }
        Err(AuthCommandError::Store(CredentialStoreError::BackendAbsent { source })) => bail!(
            "no OS keyring backend is available ({source}), so this host has no\n\
             stored credentials; credentials here come from `SYSAND_CRED_*`\n\
             environment variables"
        ),
        Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
            Err(err).context(KEYRING_LOCKED_HINT)
        }
        // Idempotent: a warning and exit 0, so cleanup scripts need not
        // swallow a failure. The CLI adds the `auth status` pointer (a
        // logout target must match the stored key exactly).
        Err(err @ AuthCommandError::NoStoredCredential { .. }) => {
            log::warn!(
                "{err}; run `sysand auth status` to list the stored logins and their exact keys"
            );
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

pub fn command_auth_status(config: &Config) -> Result<()> {
    // Status is diagnostic, so default-index resolution is lenient: an
    // ambiguous chain gets a note instead of the hard error bare
    // `login`/`logout` raise, and an invalid default simply marks nothing.
    let default_key = match resolve_default_index(config) {
        Ok(url) => IndexKey::validate(&url).ok(),
        // `resolve_default_index` errors only on an ambiguous chain
        // (more than one distinct default index).
        Err(_) => {
            let note = sysand_core::style::get_style_config().note;
            println!(
                "{note}note:{note:#} more than one default index is configured; no entry\n\
                 is marked as the default index"
            );
            None
        }
    };
    let default_key = default_key.as_ref().map(IndexKey::as_str);
    let env = collect_env_credential_entries()?;
    let status = match open_cli_credential_store() {
        Ok(store) => match do_auth_status(&store, env, default_key) {
            Ok(status) => status,
            Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
                return Err(err).context(KEYRING_LOCKED_HINT);
            }
            Err(err) => return Err(err.into()),
        },
        // Could not open the store: degrade to the env-only view like an
        // absent backend, keeping the env entries' default-index marking.
        Err(err) => {
            let mut status = assemble_auth_status(Vec::new(), env, chrono::Utc::now(), default_key);
            status.stored = StoredCredentialsStatus::BackendUnavailable {
                reason: err.to_string(),
            };
            status
        }
    };
    render_auth_status(&status);
    Ok(())
}

/// Collect `SYSAND_CRED_*` URL-pattern variables as status entries,
/// applying the same strict validation as the eager auth-policy build:
/// a malformed configuration (label-less name, incomplete group) is an
/// error here too, with the identical message, so `auth status` never
/// reports less than any credential-using command would reject. Only
/// pattern variables are shown; the companion variables hold secrets.
fn collect_env_credential_entries() -> Result<Vec<EnvCredentialEntry>> {
    let groups = crate::cred_env::validated_env_groups()?;
    let mut entries: Vec<EnvCredentialEntry> = groups
        .patterns
        .into_iter()
        .map(|(stem, pattern)| EnvCredentialEntry {
            label: format!("{}{stem}", crate::cred_env::ENV_PREFIX),
            pattern,
            // Set by status assembly when a default index is known.
            applies_to_default: false,
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(entries)
}

/// Days-to-expiry at or under which the `expires in N days` qualifier is
/// highlighted as a warning.
const EXPIRES_SOON_DAYS: i64 = 7;

/// The ` (expired)` / ` (expires in N days)` qualifier for an expiry
/// timestamp, styled through the house tokens (red for expired, yellow
/// when expiry is close). Empty when nothing is known.
fn expiry_qualifier(expired: bool, expires_in_days: Option<i64>) -> String {
    let style = sysand_core::style::get_style_config();
    if expired {
        let error = style.error;
        return format!(" {error}(expired){error:#}");
    }
    let text = match expires_in_days {
        // `num_days` truncates, so a not-yet-expired token with under a day
        // left reports 0; do not render "expires in 0 days".
        Some(0) => "(expires in less than a day)".to_string(),
        Some(1) => "(expires in 1 day)".to_string(),
        Some(days) => format!("(expires in {days} days)"),
        None => return String::new(),
    };
    if expires_in_days.is_some_and(|days| days <= EXPIRES_SOON_DAYS) {
        let warn = style.warn;
        format!(" {warn}{text}{warn:#}")
    } else {
        format!(" {text}")
    }
}

/// The per-entry validation claim: the same scoped wording `auth login`
/// prints, from the surfaces a validating login recorded. Dim when
/// validated (secondary detail); warn when not (the security-relevant
/// case: nothing ever exercised this credential). Unknown surfaces
/// render as their stored string.
fn validation_claim(validated: &[sysand_core::credential_store::ValidatedSurface]) -> String {
    let style = sysand_core::style::get_style_config();
    if validated.is_empty() {
        let warn = style.warn;
        format!("{warn}not validated{warn:#}")
    } else {
        let surfaces: Vec<&str> = validated.iter().map(|surface| surface.as_str()).collect();
        let dim = style.dim;
        format!("{dim}validated ({}){dim:#}", surfaces.join(", "))
    }
}

/// Render the unified status view: stored entries tagged `Stored` (key in
/// the exact form `sysand auth logout <key>` accepts) and env entries
/// tagged `Env`. Never any secret. A source with nothing to show is
/// omitted; the combined negative prints only when neither source has
/// anything; the backend-unavailable note always prints.
///
/// Styling goes through `anstream::println`, which strips it on
/// non-terminal stdout and under `NO_COLOR`, so piped output is exactly
/// the plain text the CLI tests assert on.
fn render_auth_status(status: &AuthStatus) {
    let style = sysand_core::style::get_style_config();
    let tag = style.header;
    let name = style.literal;
    let warn = style.warn;
    let dim = style.dim;
    let default_marker = |applies: bool| {
        if applies {
            format!("  {dim}(default index){dim:#}")
        } else {
            String::new()
        }
    };
    let none: Vec<StoredCredentialStatus> = Vec::new();
    let stored = match &status.stored {
        StoredCredentialsStatus::BackendUnavailable { reason } => {
            let note = style.note;
            println!(
                "{note}note:{note:#} no OS keyring backend is available ({reason}); showing\n\
                 `SYSAND_CRED_*` environment credentials only"
            );
            &none
        }
        StoredCredentialsStatus::Available(stored) => stored,
    };
    for (position, entry) in stored.iter().enumerate() {
        if position > 0 {
            println!();
        }
        println!(
            "{tag}{:>12}{tag:#} {name}{}{name:#}  {}{}",
            "Stored",
            entry.key,
            validation_claim(&entry.validated),
            default_marker(entry.applies_to_default)
        );
        println!("{:>12} {dim}covers:{dim:#} {}", ' ', entry.globs.join(", "));
        if let Some(subject) = &entry.subject {
            println!(
                "{:>12} {dim}subject:{dim:#} {} {}",
                ' ', subject.kind, subject.name
            );
        }
        if let Some(prefix) = &entry.token_prefix {
            println!("{:>12} {dim}token prefix:{dim:#} {prefix}", ' ');
        }
        if let Some(expires_at) = &entry.expires_at {
            println!(
                "{:>12} {dim}expires:{dim:#} {}{}",
                ' ',
                sysand_core::utils::format_expiry_utc(expires_at),
                expiry_qualifier(entry.expired, entry.expires_in_days)
            );
        }
        if !entry.shadowed_by.is_empty() {
            println!(
                "{:>12} {warn}shadowed by:{warn:#} {}",
                ' ',
                entry.shadowed_by.join(", ")
            );
        }
    }
    for entry in &status.env {
        println!(
            "{tag}{:>12}{tag:#} {name}{}{name:#}  {}{}",
            "Env",
            entry.label,
            entry.pattern,
            default_marker(entry.applies_to_default)
        );
    }
    if stored.is_empty() && status.env.is_empty() {
        println!(
            "No credentials configured (no stored credentials, no `SYSAND_CRED_*` variables)."
        );
    }
}

/// Build the read auth policy for `auth whoami`'s discovery fetch (and
/// its env bearer map), tolerantly: unlike the strict validation shared
/// by `run_cli` and `auth status`, a malformed `SYSAND_CRED_*` group is
/// skipped with a debug log, because `whoami` must stay usable against a
/// private index even when an unrelated group is malformed.
fn lenient_env_auth_policy() -> Result<StandardHTTPAuthentication> {
    let groups = crate::cred_env::collect_env_groups();

    let mut builder = StandardHTTPAuthenticationBuilder::new();
    for (stem, pattern) in &groups.patterns {
        // Pre-compile each pattern individually: `build` below fails
        // wholesale on one bad glob, and one bad group must not hide the
        // other credentials.
        let mut check: GlobMapBuilder<()> = GlobMapBuilder::new();
        check.add(pattern, ());
        if check.build().is_err() {
            log::debug!("skipping SYSAND_CRED_{stem}: invalid URL glob pattern");
            continue;
        }
        if let (Some(user), Some(password)) = (
            groups.basic_users.get(stem),
            groups.basic_passwords.get(stem),
        ) {
            builder.add_basic_auth(pattern, user, password);
        }
        if let Some(token) = groups.bearer_tokens.get(stem) {
            builder.add_bearer_auth(pattern, token, stem);
        }
    }
    builder
        .build()
        .context("could not compile `SYSAND_CRED_*` URL patterns")
}

pub fn command_auth_whoami(
    index_url: Option<String>,
    config: &Config,
    client: &reqwest_middleware::ClientWithMiddleware,
    runtime: &tokio::runtime::Runtime,
) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(config)?,
    };
    // Validate before any store or network access, and echo the resolved
    // index so a configured default cannot be targeted silently. Printed,
    // not logged: `--quiet` must not hide it.
    let key = IndexKey::validate(&target)?;
    let header = sysand_core::style::get_style_config().header;
    println!(
        "{header}{:>12}{header:#} identity on index `{key}`",
        "Checking"
    );

    let env_policy = lenient_env_auth_policy()?;
    let env_bearers = env_policy
        .publish_bearer_auth_map()
        .context("could not compile `SYSAND_CRED_*` URL patterns")?;
    // One store read serves both the discovery fetch and credential
    // selection (at most one keychain touch per command); on a locked
    // keyring this is the single unlock prompt.
    let store = open_cli_credential_store().context("could not open the credential store")?;
    let records = match store.list() {
        Ok(records) => records,
        // No keyring backend: only env credentials can apply. A locked
        // backend is a hard error instead: "no credential, run login"
        // would be the wrong remediation for an unlockable store.
        Err(CredentialStoreError::BackendAbsent { .. }) => Vec::new(),
        Err(err @ CredentialStoreError::BackendDenied { .. }) => {
            return Err(err).context(KEYRING_LOCKED_HINT);
        }
        Err(err) => return Err(err.into()),
    };
    // Preloaded with the records read above: the discovery policy shares
    // that single read and never touches the store itself.
    let discovery_policy = CliAuthPolicy::preloaded(env_policy, &records);

    let outcome = match do_auth_whoami(
        &records,
        &key,
        &env_bearers,
        &discovery_policy,
        client,
        runtime,
    ) {
        Ok(outcome) => outcome,
        // Core states the condition; the CLI adds the `auth login`
        // remediation, with `SYSAND_CRED_*` second as the CI path.
        Err(err @ AuthCommandError::NoWhoamiCredential { .. }) => {
            let AuthCommandError::NoWhoamiCredential { index, .. } = &err else {
                unreachable!()
            };
            return Err(anyhow::anyhow!(
                "{err}\nrun `sysand auth login {index}` to store a credential;\n\
                 in CI, set `SYSAND_CRED_*` environment variables instead"
            ));
        }
        Err(err) => return Err(err.into()),
    };

    // Name the source: env shadows stored, so the right remediation
    // depends on where the credential came from.
    match &outcome.source {
        WhoamiCredentialSource::Env { label } => {
            println!(
                "{header}{:>12}{header:#} credential from `SYSAND_CRED_{label}`",
                "Using"
            );
        }
        WhoamiCredentialSource::Stored { key } => {
            println!(
                "{header}{:>12}{header:#} stored credential for `{key}`",
                "Using"
            );
        }
    }

    match outcome.verdict {
        WhoamiVerdict::Identified { identity } => {
            match identity {
                Some(identity) => {
                    println!(
                        "{header}{:>12}{header:#} {} {}",
                        "Subject", identity.subject.kind, identity.subject.name
                    );
                    if let Some(token_name) = &identity.token_name {
                        println!("{header}{:>12}{header:#} {token_name}", "Token name");
                    }
                    if let Some(prefix) = &identity.token_prefix {
                        println!("{header}{:>12}{header:#} {prefix}", "Token prefix");
                    }
                    if let Some(expires_at) = identity.expires_at {
                        let now = chrono::Utc::now();
                        println!(
                            "{header}{:>12}{header:#} {}{}",
                            "Expires",
                            sysand_core::utils::format_expiry_utc(&expires_at),
                            expiry_qualifier(expires_at < now, Some((expires_at - now).num_days()))
                        );
                    }
                }
                None => println!(
                    "The credential was accepted, but the identity response\n\
                     could not be parsed."
                ),
            }
            Ok(())
        }
        WhoamiVerdict::Rejected => {
            let remediation = match &outcome.source {
                WhoamiCredentialSource::Env { label } => {
                    format!("rotate or unset `SYSAND_CRED_{label}_BEARER_TOKEN`")
                }
                WhoamiCredentialSource::Stored { key } => {
                    format!("re-run `sysand auth login {key}`")
                }
            };
            bail!(
                "the index API (`{}`) rejected the credential (HTTP 401); {remediation}",
                outcome.whoami_url
            );
        }
        // Covers transport failures and answers without a usable identity
        // (redirect, 429, unexpected status): "could not get an identity"
        // reads correctly for all of them, unlike "could not reach".
        WhoamiVerdict::Unreachable { detail } => bail!(
            "could not get an identity from the index API (`{}`): {detail}",
            outcome.whoami_url
        ),
    }
}

#[cfg(test)]
#[path = "./auth_tests.rs"]
mod tests;
