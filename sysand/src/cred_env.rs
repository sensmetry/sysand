// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Classification of `SYSAND_CRED_*` environment variables.
//!
//! Every consumer of these variables (the eager auth-policy build in
//! `run_cli`, the lenient policy for `auth whoami`, and the `auth status`
//! listing) must agree on which suffixes are reserved for secrets and how
//! the group stem is derived. This module is the single place that
//! knowledge lives: a new suffix is added here and nowhere else. What each
//! consumer does with a malformed or incomplete group (fail hard, skip
//! with a diagnostic, list anyway) stays with the consumer.

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

/// Split a `SYSAND_CRED_*` variable name into its group stem and role.
/// Returns `None` for variables outside the `SYSAND_CRED_` namespace.
pub(crate) fn classify(name: &str) -> Option<(String, CredEnvRole)> {
    let rest = name.strip_prefix(ENV_PREFIX)?;
    for (suffix, role) in SECRET_SUFFIXES {
        if let Some(stem) = rest.strip_suffix(suffix) {
            return Some((stem.to_owned(), role));
        }
    }
    Some((rest.to_owned(), CredEnvRole::Pattern))
}

/// `SYSAND_CRED_*` variables read from the process environment, grouped
/// by role and keyed by group stem.
#[derive(Debug, Default)]
pub(crate) struct CredEnvGroups {
    pub(crate) patterns: HashMap<String, String>,
    pub(crate) basic_users: HashMap<String, String>,
    pub(crate) basic_passwords: HashMap<String, String>,
    pub(crate) bearer_tokens: HashMap<String, String>,
}

/// Collect every `SYSAND_CRED_*` variable from the process environment
/// into its role map.
pub(crate) fn collect_env_groups() -> CredEnvGroups {
    let mut groups = CredEnvGroups::default();
    for (name, value) in std::env::vars() {
        let Some((stem, role)) = classify(&name) else {
            continue;
        };
        let map = match role {
            CredEnvRole::Pattern => &mut groups.patterns,
            CredEnvRole::BasicUser => &mut groups.basic_users,
            CredEnvRole::BasicPass => &mut groups.basic_passwords,
            CredEnvRole::BearerToken => &mut groups.bearer_tokens,
        };
        map.insert(stem, value);
    }
    groups
}

#[cfg(test)]
#[path = "./cred_env_tests.rs"]
mod tests;
