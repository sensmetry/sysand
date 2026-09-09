// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Document-level edits of a project's `usage` entries.
//!
//! Unlike `add`/`remove`, which round-trip `.project.json` through the typed
//! model (dropping keys the model does not know and re-ordering the rest),
//! the operations here edit the JSON document itself, so every other key and
//! the key order survive verbatim. `serde_json` is built with
//! `preserve_order`, which is what makes this possible.

use thiserror::Error;

use crate::{add::expand_sysand_purl_shorthand, model::InterchangeProjectValidationError};

/// Outcome of a constraint edit, so callers can report precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintChange {
    /// The usage was present and its constraint now differs.
    Replaced {
        old: Option<String>,
        new: Option<String>,
    },
    /// The usage was present and already carried exactly this constraint.
    /// The document was not modified.
    Unchanged { constraint: Option<String> },
    /// No usage matched the resource. The document was not modified.
    NotFound,
}

#[derive(Debug, Error)]
pub enum SetConstraintError {
    #[error("`{0}` is not a valid version requirement")]
    InvalidConstraint(String, #[source] semver::Error),
    #[error(transparent)]
    MalformedUsage(#[from] InterchangeProjectValidationError),
    #[error("`.project.json` is not a JSON object")]
    NotAnObject,
    #[error("`usage` is present in `.project.json` but is not an array")]
    UsageNotAnArray,
    #[error("`{resource}` is declared {count} times in `.project.json`; refusing to guess")]
    Ambiguous { resource: String, count: usize },
}

const USAGE_KEY: &str = "usage";
const RESOURCE_KEY: &str = "resource";
const VERSION_CONSTRAINT_KEY: &str = "versionConstraint";

/// Expand `resource` the way `add` does: `publisher/name` becomes the
/// `pkg:sysand/` PURL, anything else is used as given.
fn expand_resource(resource: &str) -> Result<String, SetConstraintError> {
    match expand_sysand_purl_shorthand(resource) {
        Ok(Some(purl)) => Ok(purl),
        Ok(None) => Ok(resource.to_owned()),
        Err(source) => Err(
            InterchangeProjectValidationError::MalformedUsageSysandPurl {
                iri: resource.to_owned(),
                source,
            }
            .into(),
        ),
    }
}

/// Set (or clear, with `None`) the `versionConstraint` of the `resource`
/// usage naming `resource`, editing `doc` in place.
///
/// Only the one `versionConstraint` value is touched: every other key, every
/// unknown key, and the document's key order are preserved verbatim. When a
/// constraint is added to a usage that had none, the key is appended after
/// the usage's existing keys.
///
/// `resource` is matched as `add` matches it: the `publisher/name` shorthand
/// is expanded and then compared as a plain string. Returns the expanded
/// resource alongside the change so callers can report what was matched.
///
/// The constraint is validated as a semver requirement, and a resource
/// declared more than once is refused, before anything is modified.
pub fn do_set_usage_constraint(
    doc: &mut serde_json::Value,
    resource: &str,
    constraint: Option<&str>,
) -> Result<(String, ConstraintChange), SetConstraintError> {
    let resource = expand_resource(resource)?;
    if let Some(constraint) = constraint {
        semver::VersionReq::parse(constraint)
            .map_err(|e| SetConstraintError::InvalidConstraint(constraint.to_owned(), e))?;
    }

    let serde_json::Value::Object(root) = doc else {
        return Err(SetConstraintError::NotAnObject);
    };
    let usages = match root.get_mut(USAGE_KEY) {
        None => return Ok((resource, ConstraintChange::NotFound)),
        Some(serde_json::Value::Array(usages)) => usages,
        Some(_) => return Err(SetConstraintError::UsageNotAnArray),
    };

    // Only `Resource` usages carry a `resource` key, so directory and kpar
    // path usages never match.
    let mut matches = usages.iter_mut().filter_map(|usage| match usage {
        serde_json::Value::Object(usage)
            if usage.get(RESOURCE_KEY).and_then(serde_json::Value::as_str) == Some(&resource) =>
        {
            Some(usage)
        }
        _ => None,
    });
    let Some(usage) = matches.next() else {
        return Ok((resource, ConstraintChange::NotFound));
    };
    let count = 1 + matches.count();
    if count > 1 {
        return Err(SetConstraintError::Ambiguous { resource, count });
    }

    let old = match usage.get(VERSION_CONSTRAINT_KEY) {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => value.as_str().map(str::to_owned),
    };
    if old.as_deref() == constraint {
        return Ok((resource, ConstraintChange::Unchanged { constraint: old }));
    }

    match constraint {
        Some(constraint) => {
            // Assigning through the existing entry keeps its position;
            // inserting a new key appends it.
            usage.insert(
                VERSION_CONSTRAINT_KEY.to_owned(),
                serde_json::Value::String(constraint.to_owned()),
            );
        }
        None => {
            // `remove` is `swap_remove` under `preserve_order`; keep the order
            // of the remaining keys.
            usage.shift_remove(VERSION_CONSTRAINT_KEY);
        }
    }

    Ok((
        resource,
        ConstraintChange::Replaced {
            old,
            new: constraint.map(str::to_owned),
        },
    ))
}

/// [`do_set_usage_constraint`] on the `.project.json` of a local project,
/// written back in sysand's pretty format only when the document changed.
#[cfg(feature = "filesystem")]
pub fn do_set_usage_constraint_local(
    project: &mut crate::project::local_src::LocalSrcProject,
    resource: &str,
    constraint: Option<&str>,
) -> Result<(String, ConstraintChange), crate::project::local_src::EditInfoError<SetConstraintError>>
{
    project.edit_info_document(|doc| do_set_usage_constraint(doc, resource, constraint))
}

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
