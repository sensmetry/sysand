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
    model::{IndexName, IndexPublisher, InterchangeProjectUsageRaw},
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
    /// The editor's text: a `model.toml` `dir`, `kpar-path`, `kpar-url`,
    /// `git` or opt-in entry has no constraint to set.
    #[error(transparent)]
    TomlUsageHasNoConstraint(crate::manifest::edit::EditError),
    /// A `model.toml` usage is addressed by identity, and an IRI that is
    /// not a `pkg:sysand` PURL names none -- a `kpar-url` entry is
    /// declared with one, but has no constraint either way.
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

/// What [`do_set_usage_constraint_local`] and
/// [`do_set_index_usage_constraint_local`] can fail with, on either
/// manifest format.
#[cfg(feature = "filesystem")]
#[derive(Debug, Error)]
pub enum SetConstraintLocalError {
    #[error(transparent)]
    Project(crate::project::local_src::LocalSrcError),
    #[error(transparent)]
    Edit(SetConstraintError),
    #[error("failed to read the project's `model.toml`: {0}")]
    Manifest(Box<str>),
    #[error(transparent)]
    ManifestWrite(crate::project::ManifestWriteError<crate::project::local_src::LocalSrcError>),
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
    publisher: &IndexPublisher,
    name: &IndexName,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    let identifier = Identifier::from_index(publisher, name);
    let (publisher, name) = (publisher.as_str(), name.as_str());
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

    let mut exact = vec![];
    let mut same_project = None;
    for (index, usage) in usages.iter().enumerate() {
        let Ok(usage) = serde_json::from_value::<InterchangeProjectUsageRaw>(usage.clone()) else {
            continue;
        };
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
    publisher: &IndexPublisher,
    name: &IndexName,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintLocalError> {
    use crate::project::ProjectRead as _;

    if project.format().map_err(SetConstraintLocalError::Project)?
        == crate::project::ProjectFormat::Toml
    {
        return edit_toml_manifest(project, |doc| {
            do_set_index_usage_constraint_toml(doc, publisher, name, constraint)
        });
    }
    project
        .edit_info_document(|doc| do_set_index_usage_constraint(doc, publisher, name, constraint))
        .map_err(local_error)
}

/// [`do_set_usage_constraint`] on the `.project.json` of a local project,
/// written back in sysand's pretty format only when the document changed.
#[cfg(feature = "filesystem")]
pub fn do_set_usage_constraint_local(
    project: &mut crate::project::local_src::LocalSrcProject,
    resource: &str,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintLocalError> {
    use crate::project::ProjectRead as _;

    // A directory holding both manifests keeps editing `.project.json`,
    // as it did before the toml arm existed.
    if project.format().map_err(SetConstraintLocalError::Project)?
        == crate::project::ProjectFormat::Toml
    {
        return edit_toml_manifest(project, |doc| {
            do_set_usage_constraint_toml(doc, resource, constraint)
        });
    }
    project
        .edit_info_document(|doc| do_set_usage_constraint(doc, resource, constraint))
        .map_err(local_error)
}

/// The `model.toml` arm of [`do_set_usage_constraint`].
///
/// A `model.toml` usage is addressed by identity, so the resource must
/// be a `pkg:sysand` PURL (the `publisher/name` form is
/// [`do_set_index_usage_constraint_toml`]); the entry must be an index
/// usage, the only kind with a `version`. The edit goes through the
/// manifest editor, which keeps comments and every other line as they
/// were.
pub fn do_set_usage_constraint_toml(
    doc: &mut toml_edit::DocumentMut,
    resource: &str,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    use crate::manifest::common::normalized_identity;
    use crate::model::{IndexName, IndexPublisher};

    let (publisher, name) = match crate::purl::parse_sysand_purl(resource) {
        Ok(Some(parts)) => parts,
        Ok(None) => return Err(SetConstraintError::NotAnIdentity(resource.to_owned())),
        Err(source) => {
            return Err(SetConstraintError::MalformedPurl {
                iri: resource.to_owned(),
                source,
            });
        }
    };
    let (Ok(publisher), Ok(name)) = (IndexPublisher::parse(publisher), IndexName::parse(name))
    else {
        return Err(SetConstraintError::NotAnIdentity(resource.to_owned()));
    };
    set_toml_index_constraint(doc, &normalized_identity(&publisher, &name), constraint)
}

/// The `model.toml` arm of [`do_set_index_usage_constraint`]: the entry
/// with the identity, which must be an index usage, the only kind with
/// a `version`.
pub fn do_set_index_usage_constraint_toml(
    doc: &mut toml_edit::DocumentMut,
    publisher: &IndexPublisher,
    name: &IndexName,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    use crate::manifest::common::normalized_identity;
    use crate::model::{IndexName, IndexPublisher};

    let identity = format!("{}/{}", publisher.as_str(), name.as_str());
    let (Ok(publisher), Ok(name)) = (
        IndexPublisher::parse(publisher.as_str()),
        IndexName::parse(name.as_str()),
    ) else {
        return Err(SetConstraintError::NotAnIdentity(identity));
    };
    set_toml_index_constraint(doc, &normalized_identity(&publisher, &name), constraint)
}

/// Sets the `version` of the `[[inline-usages]]` entry `wanted` names.
fn set_toml_index_constraint(
    doc: &mut toml_edit::DocumentMut,
    wanted: &crate::manifest::common::NormalizedIdentity,
    constraint: &str,
) -> Result<ConstraintChange, SetConstraintError> {
    let requirement = semver::VersionReq::parse(constraint)
        .map_err(|e| SetConstraintError::InvalidConstraint(constraint.to_owned(), e))?;
    match crate::manifest::edit::usage_constraint(doc, wanted) {
        None => Ok(ConstraintChange::NotFound),
        // The entry is there and has no `version`: not an index usage.
        Some(None) => Err(SetConstraintError::TomlUsageHasNoConstraint(
            crate::manifest::edit::EditError::NotAnIndexUsage(wanted.clone()),
        )),
        Some(Some(old)) => {
            let new = requirement.to_string();
            if old == new {
                return Ok(ConstraintChange::Unchanged { constraint: old });
            }
            crate::manifest::edit::set_usage_version(doc, wanted, &requirement)
                .map_err(SetConstraintError::TomlUsageHasNoConstraint)?;
            Ok(ConstraintChange::Replaced {
                old: Some(old),
                new,
            })
        }
    }
}

/// Edits the `model.toml` of `project` in place, writing it back only when
/// the constraint changed.
#[cfg(feature = "filesystem")]
fn edit_toml_manifest(
    project: &mut crate::project::local_src::LocalSrcProject,
    edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<ConstraintChange, SetConstraintError>,
) -> Result<ConstraintChange, SetConstraintLocalError> {
    use crate::project::{ProjectMut as _, ProjectRead as _};

    let mut document = project.manifest_document().map_err(|e| match e {
        crate::project::ManifestError::Project(e) => SetConstraintLocalError::Project(e),
        crate::project::ManifestError::NoTomlManifest => {
            SetConstraintLocalError::Manifest("this project has no `model.toml` manifest".into())
        }
    })?;
    let outcome = edit(&mut document).map_err(SetConstraintLocalError::Edit)?;
    if matches!(outcome, ConstraintChange::Replaced { old: _, new: _ }) {
        project
            .put_manifest_document(&document, true)
            .map_err(SetConstraintLocalError::ManifestWrite)?;
    }
    Ok(outcome)
}

#[cfg(feature = "filesystem")]
fn local_error(
    error: crate::project::local_src::EditInfoError<SetConstraintError>,
) -> SetConstraintLocalError {
    match error {
        crate::project::local_src::EditInfoError::Project(e) => SetConstraintLocalError::Project(e),
        crate::project::local_src::EditInfoError::Edit(e) => SetConstraintLocalError::Edit(e),
    }
}

#[cfg(test)]
#[path = "./usage_tests.rs"]
mod tests;
