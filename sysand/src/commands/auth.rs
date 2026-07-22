// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! CLI wrappers for the `sysand auth` commands (design/credential-storage.md
//! sections 4, 9): default-index resolution, the OS keyring store handoff
//! to core, and user-facing rendering. Secrets never appear in any output.

use std::io::{IsTerminal, Read};
use std::sync::Arc;

use anstream::println;
use anyhow::{Context, Result, bail};
use sysand_core::{
    auth::{GlobMapBuilder, StandardHTTPAuthentication, StandardHTTPAuthenticationBuilder},
    commands::auth::{
        AuthCommandError, AuthLoginNotice, AuthLoginOutcome, AuthStatus, EnvCredentialEntry,
        ProbeSurface, StoredCredentialStatus, StoredCredentialsStatus, WhoamiCredentialSource,
        WhoamiVerdict, assemble_auth_status, do_auth_login, do_auth_logout, do_auth_status,
        do_auth_whoami, validated_index_key,
    },
    config::Config,
    credential_store::{CredentialStore, CredentialStoreError},
};

use chrono::{DateTime, Utc};

use crate::{CliAuthPolicy, DEFAULT_INDEX_URL, credential_store::open_cli_credential_store};

const KEYRING_LOCKED_HINT: &str = "unlock your OS keyring and retry, or provide credentials via \
     `SYSAND_CRED_*` environment variables";

/// Resolve the single index a bare `sysand auth login` / `auth logout`
/// targets (design/credential-storage.md section 4): the
/// `SYSAND_DEFAULT_INDEX` environment override (comma-delimited) when
/// set, else a `default = true` index from configuration, else the
/// built-in [`DEFAULT_INDEX_URL`]. If the consulted stage yields more
/// than one distinct URL, the target is ambiguous and an explicit URL is
/// required. Read from the environment here rather than a per-subcommand
/// `--default-index` flag: a flag would only duplicate the positional
/// index argument.
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
    // unanchorable-template target fails without prompting.
    let key = validated_index_key(&target)?;
    // Echo the resolved index before reading the secret, on both the
    // prompt and the stdin path, so a project-configured default cannot
    // be targeted silently (design/credential-storage.md section 4).
    // Printed, not logged: `--quiet` must not hide it.
    let header = sysand_core::style::get_style_config().header;
    println!("{header}{:>12}{header:#} to index `{key}`", "Logging in");

    let secret = read_token(&key, token_stdin)?;

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    let index_key = key.clone();
    let outcome = do_auth_login(&mut store, &key, secret, client, runtime, |notice| {
        match notice {
            // Printed before the store write happens.
            AuthLoginNotice::ReplacingExisting { key } => {
                let header = sysand_core::style::get_style_config().header;
                println!(
                    "{header}{:>12}{header:#} existing credential for `{key}`",
                    "Replacing"
                );
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
    });
    match outcome {
        Ok(AuthLoginOutcome::Stored {
            key,
            globs,
            validated,
        }) => {
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
            println!("{header}{:>12}{header:#} {}", "Covers", globs.join(", "));
            Ok(())
        }
        Ok(AuthLoginOutcome::BackendUnavailable { key, globs, reason }) => {
            // No-keyring host (design/credential-storage.md section 9):
            // refuse to persist, print the exact `SYSAND_CRED_*` lines to
            // set instead. The secret is never echoed: stdout on such
            // hosts typically lands in captured CI job logs.
            println!(
                "No OS keyring backend is available ({reason}), so the credential \
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

/// Remediation message for the platform credential-store cap on `auth login`
/// (Windows ~2.5 KB). The core store reports only the condition; naming the
/// `sysand auth` commands to fix it belongs here in the CLI. Points at
/// `auth status`/`auth logout` and lists already-expired logins as the
/// obvious drop candidates. With no stored logins (a single oversized token)
/// there is nothing to remove, so it does not suggest removing one.
fn blob_full_message(stored: &[(&str, Option<DateTime<Utc>>)], now: DateTime<Utc>) -> String {
    let expired: Vec<&str> = stored
        .iter()
        .filter(|(_, at)| at.is_some_and(|at| at < now))
        .map(|(key, _)| *key)
        .collect();
    let mut message = if stored.is_empty() {
        "the token is too large for this platform's credential store \
         (Windows ~2.5 KB limit); use a smaller token"
            .to_string()
    } else {
        "the credential store is full (Windows ~2.5 KB limit); remove a login with \
         `sysand auth logout <index>` (run `sysand auth status` to list them) or use \
         a smaller token"
            .to_string()
    };
    if !expired.is_empty() {
        message.push_str("\nthese stored logins have expired and are safe to remove:");
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

pub fn command_auth_logout(index_url: Option<String>, config: &Config) -> Result<()> {
    let target = match index_url {
        Some(url) => url,
        None => resolve_default_index(config)?,
    };
    // Echo the resolved index (normalized to the stored-key form when
    // possible) so a configured default cannot be targeted silently.
    // Printed, not logged: `--quiet` must not hide it.
    let echo = validated_index_key(&target).unwrap_or_else(|_| target.clone());
    let header = sysand_core::style::get_style_config().header;
    println!(
        "{header}{:>12}{header:#} from index `{echo}`",
        "Logging out"
    );

    let mut store = open_cli_credential_store().context("could not open the credential store")?;
    match do_auth_logout(&mut store, &target) {
        Ok(key) => {
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

pub fn command_auth_status(config: &Config) -> Result<()> {
    // Resolve the default index only to mark the entries that apply to
    // it. Status is a diagnostic view, so resolution is lenient: an
    // ambiguous chain gets a note instead of the hard error bare
    // `login`/`logout` raise, and a default that is not a valid HTTP(S)
    // index key (for example `file://`) simply marks nothing.
    let default_key = match resolve_default_index(config) {
        Ok(url) => validated_index_key(&url).ok(),
        // `resolve_default_index` errors only on an ambiguous chain
        // (more than one distinct default index).
        Err(_) => {
            let note = sysand_core::style::get_style_config().note;
            println!(
                "{note}note:{note:#} more than one default index is configured; no entry \
                 is marked as the default index"
            );
            None
        }
    };
    let env = collect_env_credential_entries();
    let status = match open_cli_credential_store() {
        Ok(store) => match do_auth_status(&store, env, default_key.as_deref()) {
            Ok(status) => status,
            Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
                return Err(err).context(KEYRING_LOCKED_HINT);
            }
            Err(err) => return Err(err.into()),
        },
        // Could not even open the store (no per-user lock path): degrade
        // to the env-only view like an absent backend. Assembly still
        // runs so env entries keep their default-index marking.
        Err(err) => {
            let mut status =
                assemble_auth_status(Vec::new(), env, chrono::Utc::now(), default_key.as_deref());
            status.stored = StoredCredentialsStatus::BackendUnavailable {
                reason: err.to_string(),
            };
            status
        }
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
            // Set by status assembly when a default index is known.
            applies_to_default: false,
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

/// Days-to-expiry at or under which the `expires in N days` qualifier is
/// highlighted as a warning.
const EXPIRES_SOON_DAYS: i64 = 7;

/// Render an expiry timestamp for display, without chrono's sub-second
/// noise (`11:39:28.149443` reads as `11:39:28`).
fn format_expiry_timestamp(expires_at: &chrono::DateTime<chrono::Utc>) -> String {
    expires_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

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
/// case: nothing ever exercised this credential).
fn validation_claim(validated: &[String]) -> String {
    let style = sysand_core::style::get_style_config();
    if validated.is_empty() {
        let warn = style.warn;
        format!("{warn}not validated{warn:#}")
    } else {
        let dim = style.dim;
        format!("{dim}validated ({}){dim:#}", validated.join(", "))
    }
}

/// Render the unified status view: stored entries tagged `Stored` (key in
/// the exact form `sysand auth logout <key>` accepts), env entries tagged
/// `Env`. Never any secret.
///
/// The tags sit right-aligned in the CLI's 12-column gutter; the sublines
/// of an entry keep their `label: value` form (unlike `auth whoami`, which
/// puts each field's label in the gutter: here the fields are subordinate
/// to a tagged entry, and the hierarchy would be lost) and are indented to
/// the gutter like other multi-line log messages, with the sublabels
/// dimmed so the values carry the visual weight. Multiple stored entries
/// are separated by a blank line; each entry's header line carries its
/// validation claim after the key, mirroring the `Env` lines' two-space
/// label/pattern separation.
///
/// Entries that apply to the default index (which is what bare commands
/// use) carry a dim `(default index)` annotation at the end of their
/// header line, two spaces after the validation claim (stored) or the
/// pattern (env).
///
/// A source with nothing to show is simply omitted; only when neither
/// source has anything does a single combined negative print. The
/// backend-unavailable note is information, not a negative, and always
/// prints when the keyring backend is unusable.
///
/// Styling reuses the house tokens (`sysand_core::style`) and goes through
/// `anstream::println`, which strips it on non-terminal stdout and under
/// `NO_COLOR`, so piped output stays exactly the plain text the CLI tests
/// assert on (the alignment spaces are content, the styling is not).
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
                "{note}note:{note:#} no usable OS keyring backend ({reason}); showing \
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
        println!(
            "{:>12} {dim}patterns:{dim:#} {}",
            ' ',
            entry.globs.join(", ")
        );
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
                format_expiry_timestamp(expires_at),
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
        println!("No credentials configured (no stored logins, no `SYSAND_CRED_*` variables).");
    }
}

/// Build the read auth policy for `auth whoami`'s discovery fetch (and its
/// env bearer map), tolerantly: unlike the eager policy build in
/// `run_cli`, a malformed `SYSAND_CRED_*` group (an incomplete pair, an
/// invalid glob pattern) is skipped with a debug log instead of failing
/// the command, because the `auth` commands must stay usable to diagnose
/// exactly those (same stance as `auth status`).
fn lenient_env_auth_policy() -> Result<StandardHTTPAuthentication> {
    let mut patterns = std::collections::HashMap::new();
    let mut users = std::collections::HashMap::new();
    let mut passwords = std::collections::HashMap::new();
    let mut tokens = std::collections::HashMap::new();
    for (key, value) in std::env::vars() {
        if let Some(rest) = key.strip_prefix("SYSAND_CRED_") {
            if let Some(stem) = rest.strip_suffix("_BASIC_USER") {
                users.insert(stem.to_owned(), value);
            } else if let Some(stem) = rest.strip_suffix("_BASIC_PASS") {
                passwords.insert(stem.to_owned(), value);
            } else if let Some(stem) = rest.strip_suffix("_BEARER_TOKEN") {
                tokens.insert(stem.to_owned(), value);
            } else {
                patterns.insert(rest.to_owned(), value);
            }
        }
    }

    let mut builder = StandardHTTPAuthenticationBuilder::new();
    for (stem, pattern) in &patterns {
        // Pre-compile each pattern individually: `build` below fails
        // wholesale on one bad glob, and one bad group must not hide the
        // other credentials.
        let mut check: GlobMapBuilder<()> = GlobMapBuilder::new();
        check.add(pattern, ());
        if check.build().is_err() {
            log::debug!("skipping SYSAND_CRED_{stem}: invalid URL glob pattern");
            continue;
        }
        if let (Some(user), Some(password)) = (users.get(stem), passwords.get(stem)) {
            builder.add_basic_auth(pattern, user, password);
        }
        if let Some(token) = tokens.get(stem) {
            builder.add_bearer_auth_labeled(pattern, token, stem);
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
    let key = validated_index_key(&target)?;
    let header = sysand_core::style::get_style_config().header;
    println!(
        "{header}{:>12}{header:#} identity on index `{key}`",
        "Checking"
    );

    let env_policy = lenient_env_auth_policy()?;
    let env_bearers = env_policy
        .publish_bearer_auth_map()
        .context("could not compile `SYSAND_CRED_*` URL patterns")?;
    // Two store handles on the same underlying store: one feeds the
    // discovery policy (a private index may gate its discovery document,
    // so unlike login the fetch runs with the regular read policy), the
    // other is read for credential selection.
    let discovery_store =
        open_cli_credential_store().context("could not open the credential store")?;
    let store = open_cli_credential_store().context("could not open the credential store")?;
    let discovery_policy = CliAuthPolicy::new(env_policy, discovery_store);

    let outcome = match do_auth_whoami(
        &store,
        &target,
        &env_bearers,
        &discovery_policy,
        client,
        runtime,
    ) {
        Ok(outcome) => outcome,
        Err(AuthCommandError::Store(err @ CredentialStoreError::BackendDenied { .. })) => {
            return Err(err).context(KEYRING_LOCKED_HINT);
        }
        Err(err) => return Err(err.into()),
    };

    // Name the source: an env credential shadows a stored login, so the
    // right remediation for a bad credential depends on where it came
    // from (design/credential-storage.md section 7).
    match &outcome.source {
        WhoamiCredentialSource::Env { label: Some(label) } => {
            println!(
                "{header}{:>12}{header:#} credential from `SYSAND_CRED_{label}`",
                "Using"
            );
        }
        WhoamiCredentialSource::Env { label: None } => {
            println!(
                "{header}{:>12}{header:#} a `SYSAND_CRED_*` environment credential",
                "Using"
            );
        }
        WhoamiCredentialSource::Stored { key } => {
            println!("{header}{:>12}{header:#} stored login for `{key}`", "Using");
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
                            format_expiry_timestamp(&expires_at),
                            expiry_qualifier(expires_at < now, Some((expires_at - now).num_days()))
                        );
                    }
                }
                None => println!(
                    "The credential was accepted, but the identity response \
                     could not be parsed."
                ),
            }
            Ok(())
        }
        WhoamiVerdict::Rejected => {
            let remediation = match &outcome.source {
                WhoamiCredentialSource::Env { label: Some(label) } => {
                    format!("rotate or unset `SYSAND_CRED_{label}_BEARER_TOKEN`")
                }
                WhoamiCredentialSource::Env { label: None } => {
                    "rotate or unset the matching `SYSAND_CRED_*` variables".to_string()
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
        WhoamiVerdict::Unreachable { detail } => bail!(
            "could not reach the index API (`{}`): {detail}",
            outcome.whoami_url
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::blob_full_message;
    use chrono::{Duration, Utc};

    #[test]
    fn blob_full_first_login_does_not_suggest_removing_a_login() {
        let message = blob_full_message(&[], Utc::now());
        assert!(message.contains("use a smaller token"), "{message}");
        assert!(!message.contains("logout"), "{message}");
    }

    #[test]
    fn blob_full_names_the_status_and_logout_commands() {
        let now = Utc::now();
        let stored = [("https://a.example/", None), ("https://b.example/", None)];
        let message = blob_full_message(&stored, now);
        assert!(message.contains("sysand auth status"), "{message}");
        assert!(message.contains("sysand auth logout <index>"), "{message}");
    }

    #[test]
    fn blob_full_lists_expired_logins_as_drop_candidates() {
        let now = Utc::now();
        let stored = [
            ("https://live.example/", Some(now + Duration::days(30))),
            ("https://dead.example/", Some(now - Duration::days(1))),
            ("https://unknown.example/", None),
        ];
        let message = blob_full_message(&stored, now);
        assert!(
            message.contains("sysand auth logout https://dead.example/"),
            "{message}"
        );
        assert!(
            !message.contains("logout https://live.example/"),
            "{message}"
        );
        assert!(
            !message.contains("logout https://unknown.example/"),
            "{message}"
        );
    }
}
