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

use crate::{
    model::{IndexUsage, InterchangeProjectUsageRaw},
    project::utils::Identifier,
};

/// Outcome of a constraint edit, so callers can report precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintChange {
    /// The usage was present and its constraint now differs. `old` is
    /// `None` when the usage had no constraint.
    Replaced { old: Option<String>, new: String },
    /// The usage was present and already carried exactly this constraint.
    /// The document was not modified.
    Unchanged { constraint: String },
    /// No usage matched the resource. The document was not modified.
    NotFound,
}

#[derive(Debug, Error)]
pub enum SetConstraintError {
    #[error("`{0}` is not a valid version requirement")]
    InvalidConstraint(String, #[source] semver::Error),
    #[error("`.project.json` is not a JSON object")]
    NotAnObject,
    #[error("`usage` is present in `.project.json` but is not an array")]
    UsageNotAnArray,
    #[error("`{resource}` is declared {count} times in `.project.json`; refusing to guess")]
    Ambiguous { resource: String, count: usize },
    /// The identifier names a usage that exists but cannot carry a version
    /// constraint, because its kind pins a single version by construction
    #[error(
        "`{identifier}` is declared as {kind} usage, which carries no\n\
        version constraint: it always resolves to the single version found there"
    )]
    UsageCannotHoldConstraint {
        identifier: String,
        kind: &'static str,
    },
    /// The identifier names an index usage, which is named by its publisher
    /// and name instead
    #[error(
        "`{identifier}` is declared as the index usage `{publisher}/{name}`;\n\
        name it by its publisher and name"
    )]
    IndexUsageMatchedByIdentifier {
        identifier: String,
        publisher: String,
        name: String,
    },
    /// No index usage has the given spelling, but one of the same project
    /// is spelled differently
    #[error("could not find index usage `{requested}`; did you mean `{existing}`?")]
    IndexUsageSpelledDifferently { requested: String, existing: String },
    /// The project is declared as a resource usage, which is named by its IRI
    #[error("`{identifier}` is declared as a resource usage, not as an index usage")]
    NotAnIndexUsage { identifier: String },
}

const USAGE_KEY: &str = "usage";
const RESOURCE_KEY: &str = "resource";
const VERSION_CONSTRAINT_KEY: &str = "versionConstraint";

/// The kind of a usage that a lookup matched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchedKind {
    /// A `Resource` usage.
    Resource,
    /// An index usage whose [`Identifier`] is the looked-up one. It is the
    /// same project, but named by publisher and name.
    Index { publisher: String, name: String },
    /// A path usage (`Directory`, `KparPath`) whose [`Identifier`] is the
    /// looked-up one. It is the same project, but cannot hold a constraint.
    Typed(&'static str),
}

/// Find a usage identified by `identifier`. Matches by `Identifier` and considers all usage types.
/// Returns `None` if `usage` fails to parse.
fn match_usage(usage: &serde_json::Value, identifier: &str) -> Option<MatchedKind> {
    // Cheap path first: a `Resource` usage is identified by its `resource`
    // string verbatim, and no deserialization is needed to compare it.
    if let serde_json::Value::Object(object) = usage
        && let Some(resource) = object.get(RESOURCE_KEY).and_then(serde_json::Value::as_str)
    {
        return (resource == identifier).then_some(MatchedKind::Resource);
    }

    let usage: InterchangeProjectUsageRaw = serde_json::from_value(usage.clone()).ok()?;
    if Identifier::from_unvalidated_usage(&usage)?.as_str() != identifier {
        return None;
    }
    Some(match usage {
        InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher, name, ..
        }) => MatchedKind::Index { publisher, name },
        usage => MatchedKind::Typed(usage.kind_with_article()),
    })
}

/// Set the `versionConstraint` of the usage naming `resource`, editing `doc`
/// in place.
///
/// Only the one `versionConstraint` value is touched: every other key, every
/// unknown key, and the document's key order are preserved verbatim. When a
/// constraint is added to a usage that had none, the key is appended after
/// the usage's existing keys.
///
/// `resource` is taken literally and compared against each usage's
/// [`Identifier`]. Only a `Resource` usage can carry a constraint, so a
/// matched typed usage is refused with
/// [`SetConstraintError::UsageCannotHoldConstraint`].
///
/// The constraint is validated as a semver requirement, and a resource
/// declared more than once is refused, before anything is modified.
pub fn do_set_usage_constraint(
    doc: &mut serde_json::Value,
    resource: &str,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    semver::VersionReq::parse(constraint)
        .map_err(|e| SetConstraintError::InvalidConstraint(constraint.to_owned(), e))?;

    let serde_json::Value::Object(root) = doc else {
        return Err(SetConstraintError::NotAnObject);
    };
    let usages = match root.get_mut(USAGE_KEY) {
        None => return Ok(ConstraintChange::NotFound),
        Some(serde_json::Value::Array(usages)) => usages,
        Some(_) => return Err(SetConstraintError::UsageNotAnArray),
    };

    let matches: Vec<(usize, MatchedKind)> = usages
        .iter()
        .enumerate()
        .filter_map(|(index, usage)| Some((index, match_usage(usage, resource)?)))
        .collect();

    let resource_matches = matches
        .iter()
        .filter(|(_, kind)| *kind == MatchedKind::Resource)
        .count();

    let index = match (resource_matches, matches.len()) {
        (0, 0) => return Ok(ConstraintChange::NotFound),
        // Declared only as a typed usage: the same project, but no kind that
        // can hold a constraint.
        (0, _) => {
            return Err(match &matches[0].1 {
                MatchedKind::Index { publisher, name } => {
                    SetConstraintError::IndexUsageMatchedByIdentifier {
                        identifier: resource.to_owned(),
                        publisher: publisher.clone(),
                        name: name.clone(),
                    }
                }
                MatchedKind::Typed(kind) => SetConstraintError::UsageCannotHoldConstraint {
                    identifier: resource.to_owned(),
                    kind,
                },
                MatchedKind::Resource => unreachable!("no resource usage matched"),
            });
        }
        // Exactly one resource usage and nothing else of that identity.
        (1, 1) => matches[0].0,
        // Either the resource is declared twice, or it is declared both as a
        // resource and as a typed usage.
        (_, count) => {
            return Err(SetConstraintError::Ambiguous {
                resource: resource.to_owned(),
                count,
            });
        }
    };

    let serde_json::Value::Object(usage) = &mut usages[index] else {
        unreachable!("only object usages can match")
    };

    let old = match usage.get(VERSION_CONSTRAINT_KEY) {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => value.as_str().map(str::to_owned),
    };
    if old.as_deref() == Some(constraint) {
        return Ok(ConstraintChange::Unchanged {
            constraint: constraint.to_owned(),
        });
    }

    // Assigning through the existing entry keeps its position; inserting a
    // new key appends it.
    usage.insert(
        VERSION_CONSTRAINT_KEY.to_owned(),
        serde_json::Value::String(constraint.to_owned()),
    );

    Ok(ConstraintChange::Replaced {
        old,
        new: constraint.to_owned(),
    })
}

/// Set the `versionConstraint` of the index usage of `publisher`/`name`,
/// spelled exactly so, editing `doc` in place as [`do_set_usage_constraint`]
/// does.
///
/// When there is none, but the same project is declared otherwise (spelled
/// differently, or as a usage of another kind), that is an error rather than
/// [`ConstraintChange::NotFound`].
pub fn do_set_index_usage_constraint(
    doc: &mut serde_json::Value,
    publisher: &str,
    name: &str,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    semver::VersionReq::parse(constraint)
        .map_err(|e| SetConstraintError::InvalidConstraint(constraint.to_owned(), e))?;

    let serde_json::Value::Object(root) = doc else {
        return Err(SetConstraintError::NotAnObject);
    };
    let usages = match root.get_mut(USAGE_KEY) {
        None => return Ok(ConstraintChange::NotFound),
        Some(serde_json::Value::Array(usages)) => usages,
        Some(_) => return Err(SetConstraintError::UsageNotAnArray),
    };

    let identifier = Identifier::from_pub_name(publisher, name);
    let mut exact = vec![];
    let mut same_project = None;
    for (index, usage) in usages.iter().enumerate() {
        let Ok(usage) = serde_json::from_value::<InterchangeProjectUsageRaw>(usage.clone()) else {
            continue;
        };
        match &usage {
            InterchangeProjectUsageRaw::Index(IndexUsage {
                publisher: p,
                name: n,
                ..
            }) if p == publisher && n == name => exact.push(index),
            _ if Identifier::from_unvalidated_usage(&usage).is_some_and(|id| id == identifier) => {
                same_project.get_or_insert(usage);
            }
            _ => {}
        }
    }

    let index = match (exact.as_slice(), same_project) {
        ([index], _) => *index,
        ([], None) => return Ok(ConstraintChange::NotFound),
        ([], Some(usage)) => {
            return Err(match usage {
                InterchangeProjectUsageRaw::Index(IndexUsage {
                    publisher: p,
                    name: n,
                    ..
                }) => SetConstraintError::IndexUsageSpelledDifferently {
                    requested: format!("{publisher}/{name}"),
                    existing: format!("{p}/{n}"),
                },
                InterchangeProjectUsageRaw::Resource { .. } => {
                    SetConstraintError::NotAnIndexUsage {
                        identifier: identifier.into_string(),
                    }
                }
                usage => SetConstraintError::UsageCannotHoldConstraint {
                    identifier: identifier.into_string(),
                    kind: usage.kind_with_article(),
                },
            });
        }
        (exact, _) => {
            return Err(SetConstraintError::Ambiguous {
                resource: format!("{publisher}/{name}"),
                count: exact.len(),
            });
        }
    };

    let serde_json::Value::Object(usage) = &mut usages[index] else {
        unreachable!("only object usages can match")
    };
    let old = usage
        .get(VERSION_CONSTRAINT_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if old.as_deref() == Some(constraint) {
        return Ok(ConstraintChange::Unchanged {
            constraint: constraint.to_owned(),
        });
    }
    usage.insert(
        VERSION_CONSTRAINT_KEY.to_owned(),
        serde_json::Value::String(constraint.to_owned()),
    );

    Ok(ConstraintChange::Replaced {
        old,
        new: constraint.to_owned(),
    })
}

/// [`do_set_index_usage_constraint`] on the `.project.json` of a local
/// project, written back in sysand's pretty format only when the document
/// changed.
#[cfg(feature = "filesystem")]
pub fn do_set_index_usage_constraint_local(
    project: &mut crate::project::local_src::LocalSrcProject,
    publisher: &str,
    name: &str,
    constraint: &str,
) -> Result<ConstraintChange, crate::project::local_src::EditInfoError<SetConstraintError>> {
    project
        .edit_info_document(|doc| do_set_index_usage_constraint(doc, publisher, name, constraint))
}

/// [`do_set_usage_constraint`] on the `.project.json` of a local project,
/// written back in sysand's pretty format only when the document changed.
#[cfg(feature = "filesystem")]
pub fn do_set_usage_constraint_local(
    project: &mut crate::project::local_src::LocalSrcProject,
    resource: &str,
    constraint: &str,
) -> Result<ConstraintChange, crate::project::local_src::EditInfoError<SetConstraintError>> {
    project.edit_info_document(|doc| do_set_usage_constraint(doc, resource, constraint))
}

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
