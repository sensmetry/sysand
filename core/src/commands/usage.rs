// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2026 Sysand contributors <opensource@sensmetry.com>

//! Document-level edits of a project's `usage` entries.
//!
//! The json operations here edit the JSON document itself, so every other
//! key and the key order survive verbatim. `serde_json` is built with
//! `preserve_order`, which is what makes this possible.

use thiserror::Error;

#[cfg(feature = "filesystem")]
use crate::{
    context::ManifestEdit,
    manifest::project::ResolvedProject,
    model::{InterchangeProjectInfoRaw, InterchangeProjectMetadataRaw},
    project::local_src::LocalSrcProject,
};
use crate::{
    manifest::{
        common::{NormalizedIdentity, normalized_identity},
        edit::{self, EditError},
    },
    model::{IndexName, IndexPublisher, InterchangeProjectUsageRaw},
    project::utils::Identifier,
};

/// Outcome of a constraint edit.
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
    #[error("`.project.json` is not a JSON object")]
    NotAnObject,
    #[error("`usage` is present in `.project.json` but is not an array")]
    UsageNotAnArray,
    #[error("the usage at index {index} of `.project.json` is not a valid usage")]
    InvalidUsage {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
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
    /// and name
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
    /// The editor's text: a `model.toml` `dir`, `kpar-path`, `kpar-url`,
    /// `git` or opt-in entry has no constraint to set.
    #[error(transparent)]
    TomlUsageHasNoConstraint(EditError),
    /// A `model.toml` usage is addressed by identity, and an IRI that is
    /// not a `pkg:sysand` PURL names none.
    #[error("`{0}` is not a `pkg:sysand` PURL; a `model.toml` usage is named `publisher/name`")]
    NotAnIdentity(String),
    /// A `pkg:sysand` PURL that does not parse, e.g. one whose parts are
    /// not normalized
    #[error("`{iri}` is not a valid `pkg:sysand` PURL")]
    MalformedPurl {
        iri: String,
        #[source]
        source: crate::purl::SysandPurlError,
    },
}

/// Why [`SetUsageConstraint`] could not be applied.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum SetUsageConstraintError {
    #[error(transparent)]
    Edit(#[from] SetConstraintError),
    #[error("the edited usage is not one the project model reads")]
    Reread(#[source] serde_json::Error),
}

const USAGE_KEY: &str = "usage";
const VERSION_CONSTRAINT_KEY: &str = "versionConstraint";

/// The kind of a usage that a lookup matched.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchedKind {
    Resource,
    /// An index usage whose [`Identifier`] is the looked-up one. It is the
    /// same project, but named by publisher and name.
    Index {
        publisher: String,
        name: String,
    },
    /// A path usage (`Directory`, `KparPath`) whose [`Identifier`] is the
    /// looked-up one. It is the same project, but cannot hold a constraint.
    Typed(&'static str),
}

/// Parses the usage at `index` of the `usage` array.
fn parse_usage(
    index: usize,
    usage: &serde_json::Value,
) -> Result<InterchangeProjectUsageRaw, SetConstraintError> {
    serde_json::from_value(usage.clone())
        .map_err(|source| SetConstraintError::InvalidUsage { index, source })
}

/// Matches `usage` against `identifier`, for all usage types.
fn match_usage(usage: InterchangeProjectUsageRaw, identifier: &str) -> Option<MatchedKind> {
    // A `Resource` usage is identified by its `resource` string verbatim.
    if let InterchangeProjectUsageRaw::Resource {
        resource,
        version_constraint: _,
    } = &usage
    {
        return (resource == identifier).then_some(MatchedKind::Resource);
    }

    if Identifier::from_unvalidated_usage(&usage)?.as_str() != identifier {
        return None;
    }
    Some(match usage {
        InterchangeProjectUsageRaw::Index {
            publisher,
            name,
            version_constraint: _,
        } => MatchedKind::Index { publisher, name },
        usage => MatchedKind::Typed(usage.kind_with_article()),
    })
}

/// Set the `versionConstraint` of the usage naming `resource`, editing `doc`
/// in place.
///
/// Only the `versionConstraint` value is touched; every other key and the key
/// order are preserved. A new constraint is appended after the usage's
/// existing keys.
///
/// `resource` is taken literally and compared against each usage's
/// [`Identifier`]. A matched typed usage is refused with
/// [`SetConstraintError::UsageCannotHoldConstraint`].
///
/// The constraint is validated as a semver requirement, and a resource
/// declared more than once or a `usage` entry that does not parse is
/// refused, before `doc` is modified.
pub fn do_set_usage_constraint(
    doc: &mut serde_json::Value,
    resource: &str,
    constraint: &semver::VersionReq,
) -> Result<ConstraintChange, SetConstraintError> {
    let constraint = constraint.to_string();

    let serde_json::Value::Object(root) = doc else {
        return Err(SetConstraintError::NotAnObject);
    };
    let usages = match root.get_mut(USAGE_KEY) {
        None => return Ok(ConstraintChange::NotFound),
        Some(serde_json::Value::Array(usages)) => usages,
        Some(_) => return Err(SetConstraintError::UsageNotAnArray),
    };

    let mut matches: Vec<(usize, MatchedKind)> = vec![];
    for (index, usage) in usages.iter().enumerate() {
        if let Some(kind) = match_usage(parse_usage(index, usage)?, resource) {
            matches.push((index, kind));
        }
    }

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
    if old.as_deref() == Some(constraint.as_str()) {
        return Ok(ConstraintChange::Unchanged { constraint });
    }

    // Assigning through the existing entry keeps its position; inserting a
    // new key appends it.
    usage.insert(
        VERSION_CONSTRAINT_KEY.to_owned(),
        serde_json::Value::String(constraint.clone()),
    );

    Ok(ConstraintChange::Replaced {
        old,
        new: constraint,
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
    publisher: &IndexPublisher,
    name: &IndexName,
    constraint: &semver::VersionReq,
) -> Result<ConstraintChange, SetConstraintError> {
    let identifier = Identifier::from_index(publisher, name);
    let (publisher, name) = (publisher.as_str(), name.as_str());
    let constraint = constraint.to_string();

    let serde_json::Value::Object(root) = doc else {
        return Err(SetConstraintError::NotAnObject);
    };
    let usages = match root.get_mut(USAGE_KEY) {
        None => return Ok(ConstraintChange::NotFound),
        Some(serde_json::Value::Array(usages)) => usages,
        Some(_) => return Err(SetConstraintError::UsageNotAnArray),
    };

    let mut exact = vec![];
    let mut same_project = None;
    for (index, usage) in usages.iter().enumerate() {
        let usage = parse_usage(index, usage)?;
        match &usage {
            InterchangeProjectUsageRaw::Index {
                publisher: p,
                name: n,
                version_constraint: _,
            } if p == publisher && n == name => exact.push(index),
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
                InterchangeProjectUsageRaw::Index {
                    publisher: p,
                    name: n,
                    version_constraint: _,
                } => SetConstraintError::IndexUsageSpelledDifferently {
                    requested: format!("{publisher}/{name}"),
                    existing: format!("{p}/{n}"),
                },
                InterchangeProjectUsageRaw::Resource {
                    resource: _,
                    version_constraint: _,
                } => SetConstraintError::NotAnIndexUsage {
                    identifier: identifier.into_string(),
                },
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
    if old.as_deref() == Some(constraint.as_str()) {
        return Ok(ConstraintChange::Unchanged { constraint });
    }
    usage.insert(
        VERSION_CONSTRAINT_KEY.to_owned(),
        serde_json::Value::String(constraint.clone()),
    );

    Ok(ConstraintChange::Replaced {
        old,
        new: constraint,
    })
}

/// Which usage [`SetUsageConstraint`] changes.
#[cfg(feature = "filesystem")]
#[derive(Debug, Clone)]
pub enum UsageTarget {
    /// A resource usage, by its IRI; on a `model.toml` project, a
    /// `pkg:sysand` PURL naming the project.
    Resource(String),
    /// An index usage, by its publisher and name.
    Index {
        publisher: IndexPublisher,
        name: IndexName,
    },
}

/// Set the version constraint of one usage of the current project.
#[cfg(feature = "filesystem")]
#[derive(Debug, Clone)]
pub struct SetUsageConstraint {
    pub target: UsageTarget,
    pub constraint: semver::VersionReq,
}

#[cfg(feature = "filesystem")]
impl ManifestEdit for SetUsageConstraint {
    type Output = ConstraintChange;
    type Error = SetUsageConstraintError;

    fn toml(
        self,
        document: &mut toml_edit::DocumentMut,
        _project: &ResolvedProject,
    ) -> Result<ConstraintChange, SetUsageConstraintError> {
        Ok(match &self.target {
            UsageTarget::Resource(resource) => {
                do_set_usage_constraint_toml(document, resource, &self.constraint)
            }
            UsageTarget::Index { publisher, name } => {
                do_set_index_usage_constraint_toml(document, publisher, name, &self.constraint)
            }
        }?)
    }

    fn json(
        self,
        _project: &LocalSrcProject,
        info: &mut InterchangeProjectInfoRaw,
        _meta: &mut InterchangeProjectMetadataRaw,
    ) -> Result<ConstraintChange, SetUsageConstraintError> {
        let mut document = serde_json::to_value(&*info).map_err(SetUsageConstraintError::Reread)?;
        let change = match &self.target {
            UsageTarget::Resource(resource) => {
                do_set_usage_constraint(&mut document, resource, &self.constraint)
            }
            UsageTarget::Index { publisher, name } => {
                do_set_index_usage_constraint(&mut document, publisher, name, &self.constraint)
            }
        }?;
        *info = serde_json::from_value(document).map_err(SetUsageConstraintError::Reread)?;
        Ok(change)
    }
}

/// The `model.toml` arm of [`do_set_usage_constraint`].
///
/// A `model.toml` usage is addressed by identity, so the resource must
/// be a `pkg:sysand` PURL (the `publisher/name` form is
/// [`do_set_index_usage_constraint_toml`]); the entry must be an index
/// usage, the only kind with a `version`. Comments and other lines are kept.
pub fn do_set_usage_constraint_toml(
    doc: &mut toml_edit::DocumentMut,
    resource: &str,
    constraint: &semver::VersionReq,
) -> Result<ConstraintChange, SetConstraintError> {
    let wanted = match NormalizedIdentity::from_purl(resource) {
        Ok(Some(wanted)) => wanted,
        Ok(None) => return Err(SetConstraintError::NotAnIdentity(resource.to_owned())),
        Err(source) => {
            return Err(SetConstraintError::MalformedPurl {
                iri: resource.to_owned(),
                source,
            });
        }
    };
    set_toml_index_constraint(doc, &wanted, constraint)
        .map_err(SetConstraintError::TomlUsageHasNoConstraint)
}

/// The `model.toml` arm of [`do_set_index_usage_constraint`]: the entry
/// with the identity, which must be an index usage, the only kind with
/// a `version`.
pub fn do_set_index_usage_constraint_toml(
    doc: &mut toml_edit::DocumentMut,
    publisher: &IndexPublisher,
    name: &IndexName,
    constraint: &semver::VersionReq,
) -> Result<ConstraintChange, SetConstraintError> {
    set_toml_index_constraint(doc, &normalized_identity(publisher, name), constraint)
        .map_err(SetConstraintError::TomlUsageHasNoConstraint)
}

/// Sets the `version` of the `[[inline-usages]]` entry `wanted` names.
///
/// An entry with a `dir`, `kpar-path`, `kpar-url`, `git` or opt-in marker
/// has no constraint to set, and is refused, as is an entry with neither a
/// source nor a `version`.
pub(crate) fn set_toml_index_constraint(
    doc: &mut toml_edit::DocumentMut,
    wanted: &NormalizedIdentity,
    requirement: &semver::VersionReq,
) -> Result<ConstraintChange, EditError> {
    let old = match edit::usage_constraint(doc, wanted) {
        None => return Ok(ConstraintChange::NotFound),
        Some(None) => return Err(EditError::NotAnIndexUsage(wanted.clone())),
        Some(Some(old)) => old,
    };
    let new = requirement.to_string();
    if old == new {
        return Ok(ConstraintChange::Unchanged { constraint: new });
    }
    edit::set_usage_version(doc, wanted, requirement)?;
    Ok(ConstraintChange::Replaced {
        old: Some(old),
        new,
    })
}

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
