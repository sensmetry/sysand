// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Migration of Sysand's versioned files (the lockfile and the env metadata)
//! of older versions to the current version.
//!
//! A file of an older version is migrated one version at a time, by the
//! [`Migration`]s of its kind, right after it is deserialized and before it
//! is validated. Only versions whose differences can be fixed up using
//! nothing but the file itself can be migrated.

use std::collections::HashMap;

use crate::{
    model::{ProjectName, ProjectPublisher},
    project::utils::{Identifier, URN_SYSAND_PREFIX},
};

/// Migration of the entries `T` of a file from version `from` to version `to`
pub(crate) struct Migration<T> {
    pub from: &'static str,
    pub to: &'static str,
    pub migrate: fn(&mut [T]),
}

/// Whether a file of `version` can be migrated by `migrations`
pub(crate) fn is_migratable<T>(migrations: &[Migration<T>], version: &str) -> bool {
    migrations.iter().any(|m| m.from == version)
}

/// Migrate the `entries` of a `file` of `version` to version `current`.
/// `version` must be `current`, or one that [`is_migratable`], and following
/// `migrations` from it must end at `current`
pub(crate) fn migrate<T>(
    migrations: &[Migration<T>],
    current: &str,
    file: &str,
    mut version: &str,
    entries: &mut [T],
) {
    while version != current {
        let migration = migrations
            .iter()
            .find(|m| m.from == version)
            .expect("version must be migratable");
        log::debug!(
            "migrating {file} from version `{}` to `{}`",
            migration.from,
            migration.to
        );
        (migration.migrate)(entries);
        version = migration.to;
    }
}

/// An entry of a file that lists a project's identifiers and usages, the
/// latter being identifiers of other entries
pub(crate) trait IdentifiedEntry {
    fn publisher(&self) -> Option<&str>;
    fn name(&self) -> &str;
    fn identifiers_mut(&mut self) -> impl Iterator<Item = &mut String>;
    fn usages_mut(&mut self) -> impl Iterator<Item = &mut String>;
}

/// Rewrite the identifiers of `entries` that were derived from their
/// publisher and name differently (those that `is_derived`), to the
/// identifier derived from them now, and the usages of these identifiers
/// likewise. Other identifiers (e.g. IRIs) are kept, as are those of
/// entries that have no publisher, or whose publisher or name is not valid
pub(crate) fn rederive_identifiers<T: IdentifiedEntry>(
    entries: &mut [T],
    is_derived: impl Fn(&str) -> bool,
) {
    let mut rederived = HashMap::new();
    for entry in entries.iter_mut() {
        let Some(publisher) = entry.publisher() else {
            continue;
        };
        let (Ok(publisher), Ok(name)) = (
            ProjectPublisher::parse(publisher.to_owned()),
            ProjectName::parse(entry.name().to_owned()),
        ) else {
            continue;
        };
        let new = Identifier::from_project(&publisher, &name).into_string();
        for identifier in entry.identifiers_mut() {
            if is_derived(identifier) && *identifier != new {
                let old = std::mem::replace(identifier, new.clone());
                rederived.insert(old, new.clone());
            }
        }
    }
    // A usage is an identifier of another entry
    for entry in entries {
        for usage in entry.usages_mut() {
            if let Some(new) = rederived.get(usage.as_str()) {
                usage.clone_from(new);
            }
        }
    }
}

/// Whether `identifier` was derived from a publisher and name before they
/// were normalized (lockfile versions up to 0.5, env metadata versions up to
/// 0.1): `urn:sysand` identifiers held them as spelled (percent-encoded),
/// they are now normalized. `pkg:sysand` identifiers are derived the same
/// in both
pub(crate) fn is_derived_unnormalized(identifier: &str) -> bool {
    identifier.starts_with(URN_SYSAND_PREFIX)
}
