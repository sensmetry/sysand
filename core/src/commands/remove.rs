// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: © 2025 Sysand contributors <opensource@sensmetry.com>

use thiserror::Error;

use crate::{
    model::{IndexUsage, InterchangeProjectUsageRaw, InterchangeProjectValidationError},
    project::{ProjectMut, utils::Identifier},
};

#[derive(Error, Debug)]
pub enum RemoveError<ProjectError> {
    #[error(transparent)]
    Project(ProjectError),
    #[error(transparent)]
    Validation(#[from] InterchangeProjectValidationError),
    #[error("could not find usage for `{0}`")]
    UsageNotFound(Box<str>),
    #[error("could not find usage for `{publisher}/{name}`")]
    ExpUsageNotFound { publisher: String, name: String },
    /// No `Resource` usage of the IRI exists, but a typed usage of the *same
    /// project* does. Reserved as an error rather than reported as "not
    /// found", which would be untrue, so that removing by identifier can
    /// later be relaxed into removing the typed usage instead.
    #[error(
        "`{identifier}` is declared as {kind} usage, not as a resource usage;\n\
        remove it with `{remove_with}`"
    )]
    UsageIsTyped {
        identifier: String,
        kind: &'static str,
        /// The CLI command that removes it
        remove_with: String,
    },
    /// No index usage has the given spelling, but one of the same project
    /// is spelled differently
    #[error("could not find index usage `{requested}`; did you mean `{existing}`?")]
    IndexUsageSpelledDifferently { requested: String, existing: String },
    /// No index usage of the project exists, but a usage of another kind
    /// does (e.g. a legacy `pkg:sysand` resource usage)
    #[error(
        "`{identifier}` is declared as {kind} usage, not as an index usage;\n\
        remove it with `{remove_with}`"
    )]
    NotAnIndexUsage {
        identifier: String,
        kind: &'static str,
        /// The CLI command that removes it
        remove_with: String,
    },
    #[error("project is missing project information")]
    MissingInfo,
}

/// Remove the `Resource` usage naming `iri`.
///
/// Typed usages are not removed, even when they identify the same project:
/// that case is refused with [`RemoveError::UsageIsTyped`] rather than
/// reported as missing.
pub fn do_remove<P: ProjectMut>(
    project: &mut P,
    iri: String,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{iri}` from usages");

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.pop_usage(&iri);

        if popped.is_empty() {
            // The same project may be declared as a typed usage, which has the
            // same `Identifier` but is not a resource usage. Saying "not found"
            // there would be false.
            if let Some(usage) = info.usage.iter().find(|usage| {
                usage.is_typed()
                    && Identifier::from_unvalidated_usage(usage)
                        .is_some_and(|id| id.as_str() == iri)
            }) {
                return Err(RemoveError::UsageIsTyped {
                    identifier: iri,
                    kind: usage.kind_with_article(),
                    remove_with: remove_command(usage),
                });
            }
            Err(RemoveError::UsageNotFound(iri.into_boxed_str()))
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}

/// Remove the index usage of `publisher`/`name`, spelled exactly so.
///
/// When there is none, but the same project is declared otherwise (spelled
/// differently, or as a usage of another kind), that is reported rather
/// than "not found".
pub fn do_remove_index<P: ProjectMut>(
    project: &mut P,
    publisher: &str,
    name: &str,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{publisher}/{name}` from usages");

    let Some(mut info) = project.get_info().map_err(RemoveError::Project)? else {
        return Err(RemoveError::MissingInfo);
    };
    let popped: Vec<_> = info
        .usage
        .extract_if(.., |usage| {
            matches!(
                usage,
                InterchangeProjectUsageRaw::Index(IndexUsage { publisher: p, name: n, .. })
                    if p == publisher && n == name
            )
        })
        .collect();
    if !popped.is_empty() {
        project
            .put_info(&info, true)
            .map_err(RemoveError::Project)?;
        return Ok(popped);
    }

    let identifier = Identifier::from_pub_name(publisher, name);
    match info
        .usage
        .iter()
        .find(|usage| Identifier::from_unvalidated_usage(usage).is_some_and(|id| id == identifier))
    {
        Some(InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher: p,
            name: n,
            ..
        })) => Err(RemoveError::IndexUsageSpelledDifferently {
            requested: format!("{publisher}/{name}"),
            existing: format!("{p}/{n}"),
        }),
        Some(usage) => Err(RemoveError::NotAnIndexUsage {
            identifier: identifier.into_string(),
            kind: usage.kind_with_article(),
            remove_with: remove_command(usage),
        }),
        None => Err(RemoveError::ExpUsageNotFound {
            publisher: publisher.to_owned(),
            name: name.to_owned(),
        }),
    }
}

/// The CLI command that removes `usage`
fn remove_command(usage: &InterchangeProjectUsageRaw) -> String {
    /// Quote `arg` for a shell, if needed
    fn quoted(arg: &str) -> String {
        if arg.is_empty() || arg.contains(char::is_whitespace) {
            format!("\"{arg}\"")
        } else {
            arg.to_owned()
        }
    }
    match usage {
        InterchangeProjectUsageRaw::Resource { resource, .. } => {
            format!("sysand remove {}", quoted(resource))
        }
        InterchangeProjectUsageRaw::Directory {
            publisher, name, ..
        }
        | InterchangeProjectUsageRaw::KparPath {
            publisher, name, ..
        } => format!(
            "sysand experimental remove {} {}",
            quoted(publisher),
            quoted(name)
        ),
        InterchangeProjectUsageRaw::Index(IndexUsage {
            publisher, name, ..
        }) => format!("sysand remove {}", quoted(&format!("{publisher}/{name}"))),
    }
}

pub fn exp_do_remove<P: ProjectMut>(
    project: &mut P,
    publisher: &str,
    name: &str,
) -> Result<Vec<InterchangeProjectUsageRaw>, RemoveError<P::Error>> {
    let removing = "Removing";
    let header = crate::style::get_style_config().header;
    log::info!("{header}{removing:>12}{header:#} `{publisher}/{name}` from usages");

    if let Some(mut info) = project.get_info().map_err(RemoveError::Project)? {
        let popped = info.exp_pop_usage(publisher, name);

        if popped.is_empty() {
            Err(RemoveError::ExpUsageNotFound {
                publisher: publisher.to_owned(),
                name: name.to_owned(),
            })
        } else {
            project
                .put_info(&info, true)
                .map_err(RemoveError::Project)?;
            Ok(popped)
        }
    } else {
        Err(RemoveError::MissingInfo)
    }
}

#[cfg(test)]
#[path = "./remove_tests.rs"]
mod tests;
