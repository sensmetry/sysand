// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Classification of `SYSAND_CRED_*` environment variables.
//!
//! Every consumer of these variables (the eager auth-policy build in
//! `run_cli`, the lenient policy for `auth whoami`, and the `auth status`
//! listing) must agree on which suffixes are reserved for secrets and how
//! the group stem is derived. This module is the single place that
//! knowledge lives: a new suffix is added here and nowhere else. The
//! strict group validation shared by the eager policy build and `auth
//! status` lives here too ([`validated_env_groups`]); only `auth whoami`
//! keeps its own lenient reading, so it stays usable against a private
//! index even when an unrelated group is malformed.

use std::collections::HashMap;

/// Prefix shared by all credential environment variables.
pub(crate) const ENV_PREFIX: &str = "SYSAND_CRED_";

/// The role a `SYSAND_CRED_*` variable plays within its credential group.
/// A reserved suffix marks a secret-bearing variable; without one the
/// variable holds the group's URL glob pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredEnvRole {
    /// No reserved suffix: the URL glob pattern naming the group.
    Pattern,
    /// `_BASIC_USER`: the HTTP basic username.
    BasicUser,
    /// `_BASIC_PASS`: the HTTP basic password.
    BasicPass,
    /// `_BEARER_TOKEN`: the bearer token.
    BearerToken,
}

/// The reserved secret suffixes, paired with their roles. Order matters
/// only in that each variable matches at most one suffix (none of these
/// is a suffix of another).
const SECRET_SUFFIXES: [(&str, CredEnvRole); 3] = [
    ("_BASIC_USER", CredEnvRole::BasicUser),
    ("_BASIC_PASS", CredEnvRole::BasicPass),
    ("_BEARER_TOKEN", CredEnvRole::BearerToken),
];

/// How a variable name relates to the `SYSAND_CRED_` namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CredEnvName {
    /// A well-formed name: the group label (stem) plus the variable's role.
    Grouped(String, CredEnvRole),
    /// In the namespace but with no label: bare `SYSAND_CRED`, a bare
    /// role suffix such as `SYSAND_CRED_BASIC_PASS`, or an empty stem as
    /// in `SYSAND_CRED__BEARER_TOKEN`.
    MissingLabel,
}

/// Classify a `SYSAND_CRED_*` variable name. Returns `None` for variables
/// outside the `SYSAND_CRED` namespace.
pub(crate) fn classify(name: &str) -> Option<CredEnvName> {
    // The bare prefix carries no trailing underscore, so check it before
    // the prefix strip.
    if name == "SYSAND_CRED" {
        return Some(CredEnvName::MissingLabel);
    }
    let rest = name.strip_prefix(ENV_PREFIX)?;
    if rest.is_empty() {
        return Some(CredEnvName::MissingLabel);
    }
    for (suffix, role) in SECRET_SUFFIXES {
        // A bare role suffix (`SYSAND_CRED_BASIC_PASS`) leaves no label
        // and neither does an empty stem (`SYSAND_CRED__BASIC_PASS`).
        if rest == &suffix[1..] {
            return Some(CredEnvName::MissingLabel);
        }
        if let Some(stem) = rest.strip_suffix(suffix) {
            if stem.is_empty() {
                return Some(CredEnvName::MissingLabel);
            }
            return Some(CredEnvName::Grouped(stem.to_owned(), role));
        }
    }
    Some(CredEnvName::Grouped(rest.to_owned(), CredEnvRole::Pattern))
}

/// Whether `stem` is safe to suggest as a `SYSAND_CRED_<stem>` group
/// label: the bare pattern variable must classify back to this group.
/// Host-derived stems can spell a reserved secret suffix (`_basic.pass`
/// maps to `_BASIC_PASS`, making the pattern variable read as a secret)
/// or a bare role name (`basic.user` maps to `BASIC_USER`, which is
/// label-less).
pub(crate) fn stem_is_unambiguous(stem: &str) -> bool {
    classify(&format!("{ENV_PREFIX}{stem}"))
        == Some(CredEnvName::Grouped(stem.to_owned(), CredEnvRole::Pattern))
}

/// `SYSAND_CRED_*` variables read from the process environment, grouped
/// by role and keyed by group stem.
#[derive(Debug, Default)]
pub(crate) struct CredEnvGroups {
    pub(crate) patterns: HashMap<String, String>,
    pub(crate) basic_users: HashMap<String, String>,
    pub(crate) basic_passwords: HashMap<String, String>,
    pub(crate) bearer_tokens: HashMap<String, String>,
    /// Variable names in the namespace but with no label, sorted. Never
    /// part of any group; the strict consumers reject them, the lenient
    /// one skips them.
    pub(crate) missing_label: Vec<String>,
}

/// Collect every `SYSAND_CRED_*` variable from the process environment
/// into its role map.
pub(crate) fn collect_env_groups() -> CredEnvGroups {
    let mut groups = CredEnvGroups::default();
    for (name, value) in std::env::vars() {
        let (stem, role) = match classify(&name) {
            None => continue,
            Some(CredEnvName::MissingLabel) => {
                groups.missing_label.push(name);
                continue;
            }
            Some(CredEnvName::Grouped(stem, role)) => (stem, role),
        };
        let map = match role {
            CredEnvRole::Pattern => &mut groups.patterns,
            CredEnvRole::BasicUser => &mut groups.basic_users,
            CredEnvRole::BasicPass => &mut groups.basic_passwords,
            CredEnvRole::BearerToken => &mut groups.bearer_tokens,
        };
        map.insert(stem, value);
    }
    groups.missing_label.sort();
    groups
}

/// Validate collected groups the way every credential-using command
/// does: reject label-less names, secret variables without a URL
/// pattern, patterns with no complete authentication scheme, and half of
/// a basic pair. A group carrying both schemes passes with a warning.
/// Stems are checked in sorted order so the reported error is
/// deterministic when several groups are malformed. Never interpolate
/// variable values into these messages: a misnamed variable can put a
/// secret in any position.
pub(crate) fn validate_env_groups(groups: &CredEnvGroups) -> anyhow::Result<()> {
    // Reject label-less names up front: `SYSAND_CRED_BASIC_PASS` would
    // otherwise read as the pattern of a nonsense `BASIC_PASS` group.
    if let Some(name) = groups.missing_label.first() {
        anyhow::bail!(
            "{name} has no label; env credentials need one, as in SYSAND_CRED_MYINDEX\n\
             with SYSAND_CRED_MYINDEX_BASIC_USER/SYSAND_CRED_MYINDEX_BASIC_PASS or\n\
             SYSAND_CRED_MYINDEX_BEARER_TOKEN"
        );
    }

    let mut stems: Vec<&String> = [
        &groups.patterns,
        &groups.basic_users,
        &groups.basic_passwords,
        &groups.bearer_tokens,
    ]
    .into_iter()
    .flat_map(HashMap::keys)
    .collect();
    stems.sort();
    stems.dedup();

    for k in stems {
        if !groups.patterns.contains_key(k) {
            anyhow::bail!("please specify URL pattern SYSAND_CRED_{k} for credential");
        }
        let has_basic = match (groups.basic_users.get(k), groups.basic_passwords.get(k)) {
            (Some(_), Some(_)) => true,
            (None, None) => false,
            (_, _) => {
                anyhow::bail!(
                    "please specify both (or neither) of SYSAND_CRED_{k}_BASIC_USER and SYSAND_CRED_{k}_BASIC_PASS"
                );
            }
        };
        match (has_basic, groups.bearer_tokens.contains_key(k)) {
            (false, false) => {
                anyhow::bail!(
                    "SYSAND_CRED_{k} has no matching authentication scheme, please specify\n\
                     SYSAND_CRED_{k}_BASIC_USER/SYSAND_CRED_{k}_BASIC_PASS or\n\
                     SYSAND_CRED_{k}_BEARER_TOKEN"
                );
            }
            (true, true) => {
                log::warn!("SYSAND_CRED_{k} has multiple authentication schemes!");
            }
            (_, _) => {}
        }
    }
    Ok(())
}

/// Collect and strictly validate the `SYSAND_CRED_*` environment. The
/// shared entry point for every consumer that must fail on a malformed
/// credential configuration (the eager policy build in `run_cli` and
/// `auth status`).
pub(crate) fn validated_env_groups() -> anyhow::Result<CredEnvGroups> {
    let groups = collect_env_groups();
    validate_env_groups(&groups)?;
    Ok(groups)
}

#[cfg(test)]
#[path = "./cred_env_tests.rs"]
mod tests;
