// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Migration of lockfiles of older versions to [`CURRENT_LOCK_VERSION`].
//!
//! A lockfile of an older version is migrated one version at a time, by the
//! [`Migration`]s in [`MIGRATIONS`], right after it is deserialized and
//! before it is validated. Only versions whose differences can be fixed up
//! using nothing but the lockfile itself can be migrated.

use std::collections::HashMap;

use super::{CURRENT_LOCK_VERSION, Project};
use crate::{
    model::{ProjectName, ProjectPublisher},
    project::utils::{Identifier, URN_SYSAND_PREFIX},
};

/// Migration of a lockfile from version `from` to version `to`
struct Migration {
    from: &'static str,
    to: &'static str,
    migrate: fn(&mut [Project]),
}

/// Every lockfile version that can be migrated to the next one. Following
/// these from any `from` must end at [`CURRENT_LOCK_VERSION`]
const MIGRATIONS: &[Migration] = &[Migration {
    from: "0.5",
    to: "0.6",
    // Version 0.5 put `publisher` and `name` in `urn:sysand` identifiers as
    // spelled (percent-encoded), since 0.6 they are normalized. `pkg:sysand`
    // identifiers are derived the same in both
    migrate: |projects| rederive_identifiers(projects, |id| id.starts_with(URN_SYSAND_PREFIX)),
}];

/// Whether a lockfile of `version` can be migrated to [`CURRENT_LOCK_VERSION`]
pub(super) fn is_migratable(version: &str) -> bool {
    MIGRATIONS.iter().any(|m| m.from == version)
}

/// Migrate `projects` of a lockfile of `version` to [`CURRENT_LOCK_VERSION`].
/// `version` must be the current one, or one that [`is_migratable`]
pub(super) fn migrate(mut version: &str, projects: &mut [Project]) {
    while version != CURRENT_LOCK_VERSION {
        let migration = MIGRATIONS
            .iter()
            .find(|m| m.from == version)
            .expect("lockfile version must be migratable");
        log::debug!(
            "migrating lockfile from version `{}` to `{}`",
            migration.from,
            migration.to
        );
        (migration.migrate)(projects);
        version = migration.to;
    }
}

/// Rewrite the identifiers of `projects` that were derived from their
/// publisher and name differently (those that `is_derived`), to the
/// identifier derived from them now, and the usages of these identifiers
/// likewise. Other identifiers (e.g. IRIs) are kept, as are those of
/// projects that have no publisher, or whose publisher or name is not valid
fn rederive_identifiers(projects: &mut [Project], is_derived: impl Fn(&str) -> bool) {
    let mut rederived = HashMap::new();
    for project in projects.iter_mut() {
        let Some(publisher) = &project.publisher else {
            continue;
        };
        let (Ok(publisher), Ok(name)) = (
            ProjectPublisher::parse(publisher.clone()),
            ProjectName::parse(project.name.clone()),
        ) else {
            continue;
        };
        let new = Identifier::from_project(&publisher, &name).into_string();
        for identifier in &mut project.identifiers {
            if is_derived(identifier) && *identifier != new {
                let old = std::mem::replace(identifier, new.clone());
                rederived.insert(old, new.clone());
            }
        }
    }
    // A usage is an identifier of a project in the lockfile
    for project in projects {
        for usage in &mut project.usages {
            if let Some(new) = rederived.get(usage.inner()) {
                usage.0.clone_from(new);
            }
        }
    }
}

#[cfg(test)]
#[path = "./lock_migrate_tests.rs"]
mod tests;
